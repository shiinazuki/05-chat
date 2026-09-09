# 项目命令入口。`just` 列出全部命令，`just --list` 同理。
#
# Docker 相关命令拆在 docker.just 里，可选加载（文件不存在时 `import?` 静默跳过）
import? 'docker.just'
# 模板仓库自己的维护配方，生成出来的项目里没有这个文件
import? 'template.just'

#
# 从 git remote 推导「托管平台 + owner/repo」，git-cliff 用它生成 changelog 里的提交链接。
# GitHub 用 GITHUB_REPO，GitLab 用 GITLAB_REPO，两者要分开识别。
origin_url := `git remote get-url origin 2>/dev/null || true`
# git@host:owner/repo.git 与 https://host/owner/repo.git 两种写法都剥成 owner/repo
repo_slug := `git remote get-url origin 2>/dev/null | sed -E -e 's,^[^/@]+@[^:]+:,,' -e 's,^[a-z]+://[^/]+/,,' -e 's,\.git$,,' || true`
repo_host := if origin_url =~ 'gitlab' { "gitlab" } else { if origin_url =~ 'github' { "github" } else { "" } }
# 包名（本文件不做 liquid 替换，只能从 Cargo.toml 里读）
pkg := `grep -m1 '^name' Cargo.toml | sed -E 's/.*"(.*)".*/\1/'`

# 格式化该用哪条工具链，按 rust-toolchain.toml 的 channel 推导：
#   nightly / nightly-YYYY-MM-DD  -> 就用它自己
#   stable / 具体版本号           -> 退回 nightly（just install-tools 会装那份 rustfmt）
# 读不到 rust-toolchain.toml 时落到 nightly。
fmt_toolchain := ```
    channel=$(grep -m1 '^channel' rust-toolchain.toml 2>/dev/null | sed -E 's/.*"([^"]+)".*/\1/')
    case "$channel" in
        nightly*) echo "$channel" ;;
        *)        echo nightly ;;
    esac
```

# 列出所有可用命令
default:
    @just --list --unsorted

# 「你在模板仓库里，这里跑不了 cargo」的统一闸门，被下面几条常用配方依赖。
#
# 判断依据是 Cargo.toml 里还有没有 liquid 标签。用 `{%` 而不是 `{{`：
# 后者是 just 自己的插值语法。
[private]
_generated-only:
    #!/usr/bin/env bash
    set -euo pipefail
    if grep -q '{%' Cargo.toml 2>/dev/null; then
        {
            echo "✗ 这里是【模板仓库】，不是生成出来的项目——跑不了 cargo。"
            echo
            echo "  Cargo.toml 里还是 liquid 占位符，rust-toolchain.toml 的 channel"
            echo "  也不是合法工具链名。"
            echo
            echo "  模板仓库该跑的是："
            echo "      just smoke          # 生成 10 组项目并逐个跑完整检查（模板真正的 CI）"
            echo "      just smoke-full     # 19 组完整矩阵"
            echo "      just template-lint  # 检查模板仓库自身（taplo / typos / zizmor / ...）"
            echo
            echo "  想验证某个具体组合：just smoke-keep 跑完保留现场，再进那个目录跑 just ci。"
        } >&2
        exit 1
    fi

# ---------------------------------------------------------------------------
# 日常开发
# ---------------------------------------------------------------------------

[group('dev')]
[doc('快速检查代码编译')]
check: _generated-only
    cargo check --all-targets --all-features

[group('dev')]
[doc('运行程序，额外参数原样透传：just run -- --help（仅 bin 项目）')]
run *args: _generated-only
    #!/usr/bin/env bash
    set -euo pipefail
    if [ ! -f src/main.rs ]; then
        echo "没有 bin target，跳过（库项目请写一个 examples/ 再用 cargo run --example <名字>）"
        exit 0
    fi
    cargo run --all-features {{ args }}

# rustfmt 管 .rs，taplo 管 .toml（配置见 .taplo.toml），两条一起跑
[group('dev')]
[doc('格式化代码与 TOML')]
fmt: _generated-only
    cargo +{{ fmt_toolchain }} fmt --all
    taplo fmt

[group('dev')]
[doc('自动修复 clippy 能修的问题并格式化')]
fix: _generated-only
    cargo clippy --all-targets --all-features --fix --allow-dirty --allow-staged
    just fmt

[group('dev')]
[doc('启动后台实时监控 (bacon)')]
dev: _generated-only
    bacon

[group('dev')]
[doc('生成并打开 API 文档')]
doc: _generated-only
    cargo doc --no-deps --all-features --open

[group('dev')]
[doc('跑 benchmark（benches/ 下有 target 时才有意义，profile.bench 已配好优化）')]
bench *args: _generated-only
    cargo bench --all-features {{ args }}

# 用 profiling profile 采样：优化等级与 release 一致，但保留符号。
# macOS 上 cargo-flamegraph 走 dtrace，需要 sudo；也可以换 samply：
#     cargo build --profile profiling && samply record ./target/profiling/<包名>
[group('dev')]
[doc('采样生成火焰图 flamegraph.svg（仅 bin 项目；需要 cargo-flamegraph）')]
flamegraph *args: _generated-only
    #!/usr/bin/env bash
    set -euo pipefail
    # 库项目没有 bin target，改用 --bench / --example 做性能分析
    if [ ! -f src/main.rs ]; then
        echo "没有 bin target，跳过火焰图（库项目请用 --bench / --example）"
        exit 0
    fi
    cargo flamegraph --profile profiling --bin {{ pkg }} {{ args }}

# 在 ~/.cargo/config.toml 里设了共享 build.target-dir 时，改用 `cargo clean -p <包名>`
# 只清本项目。
[group('dev')]
[doc('清理编译产物与本地生成的报告')]
clean: _generated-only
    cargo clean
    # 与 .gitignore 里那几类本地产物对齐
    rm -rf coverage
    rm -f lcov.info junit.xml flamegraph.svg profile.json perf.data* *.profraw *.profdata

# ---------------------------------------------------------------------------
# 检查
# ---------------------------------------------------------------------------

# 与 CI 的 lint job 一一对应，最后一条把 rustdoc 的警告也升级成错误
[group('check')]
[doc('格式化检查 / TOML 排版 / clippy / 拼写检查 / 文档警告（与 CI 的 lint job 等价）')]
lint: _generated-only
    cargo +{{ fmt_toolchain }} fmt --all -- --check
    taplo fmt --check
    cargo clippy --all-targets --all-features -- -D warnings
    typos
    RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features --document-private-items

[group('check')]
[doc('运行测试（含 doctest）')]
test: _generated-only
    #!/usr/bin/env bash
    set -euo pipefail
    cargo nextest run --all-targets --all-features
    # nextest 不跑 doctest，有 lib target 时补一次
    if [ -f src/lib.rs ]; then
        cargo test --doc --all-features
    fi

[group('check')]
[doc('生成覆盖率报告（lcov.info）')]
coverage: _generated-only
    cargo llvm-cov nextest --all-features --lcov --output-path lcov.info

[group('check')]
[doc('生成 HTML 覆盖率报告并在浏览器里打开')]
coverage-html: _generated-only
    cargo llvm-cov nextest --all-features --html --open

[group('check')]
[doc('依赖安全与 License 检查')]
audit: _generated-only
    cargo deny check -A unmatched-bypass

# 与 CI 的 hack job 等价：逐个 feature 组合做检查，--depth 2 限制组合爆炸
[group('check')]
[doc('遍历 feature 幂集做检查（需要 cargo-hack）')]
hack: _generated-only
    cargo hack --feature-powerset --depth 2 --no-dev-deps check

# 不放进 `just ci`，两套 CI 里也没有对应的 job：cargo-machete 靠扫源码里的符号判断，
# 只在宏里用到的依赖会被误报。误报时在 Cargo.toml 里加
# [package.metadata.cargo-machete] ignored = [...] 放行。
[group('check')]
[doc('找出声明了却没被用到的依赖（需要 cargo-machete）')]
unused: _generated-only
    cargo machete

# 同样不放进 `just ci`：要和已发布的版本比对，本地没网或没发布过时没意义。
[group('check')]
[doc('检查公开 API 有没有破坏性变更（仅纯库项目；需要 cargo-semver-checks）')]
semver: _generated-only
    #!/usr/bin/env bash
    set -euo pipefail
    # bin 项目的 lib target 只服务于自己的 main.rs 与集成测试，不参与 semver 检查；
    # 项目确实要同时发布库和命令行时，把这段判断删掉。
    if [ ! -f src/lib.rs ] || [ -f src/main.rs ]; then
        echo "不是纯库项目，跳过 semver 检查"
        exit 0
    fi
    cargo semver-checks

# nightly 项目上 MSRV 检查不适用，自动转去跑 `just nll`。
[group('check')]
[doc('验证 Cargo.toml 里声明的 MSRV 真的能编译（nightly 项目改跑 nll）')]
msrv: _generated-only
    #!/usr/bin/env bash
    set -euo pipefail
    channel=$(grep -m1 '^channel' rust-toolchain.toml | sed -E 's/.*"([^"]+)".*/\1/')
    version=$(grep -m1 '^rust-version' Cargo.toml | sed -E 's/.*"([^"]+)".*/\1/' || true)
    if [ "${channel#nightly}" != "$channel" ]; then
        echo "工具链是 ${channel}：MSRV 检查不适用，改跑 NLL 兜底检查"
        exec just nll
    fi
    if [ -z "$version" ]; then
        echo "Cargo.toml 里没有 rust-version，跳过 MSRV 检查"
        exit 0
    fi
    echo "MSRV = $version"
    rustup toolchain install "$version" --profile minimal
    cargo "+$version" check --locked --all-targets --all-features

# 用同一条 nightly 编译，但把借用检查器从 Polonius 换回 stable 的 NLL，
# 拦下只有 nightly 编得过的代码。不放进 `just ci`（换 RUSTFLAGS 等于一次全量重编），
# CI 的 msrv job 每次都会跑它。
[group('check')]
[doc('用 stable 的借用检查器（NLL）编一遍，拦下只有 nightly 编得过的代码')]
nll: _generated-only
    #!/usr/bin/env bash
    set -euo pipefail
    channel=$(grep -m1 '^channel' rust-toolchain.toml | sed -E 's/.*"([^"]+)".*/\1/')
    if [ "${channel#nightly}" = "$channel" ]; then
        echo "工具链是 ${channel}，本来用的就是 NLL，无需检查"
        exit 0
    fi
    # 换个 target 目录，避免和平时 `just check` 的产物互相顶掉
    CARGO_TARGET_DIR=target/nll RUSTFLAGS=-Zpolonius=off \
        cargo check --locked --all-targets --all-features

# 从 rustc-ice-*.txt 里摘出三处关键信息：panic 消息、产生它的编译器版本、query stack。
[group('check')]
[doc('解读 rustc-ice-*.txt：哪一版编译器崩的、崩在哪、下一步怎么办')]
ice:
    #!/usr/bin/env bash
    set -uo pipefail
    shopt -s nullglob
    dumps=(rustc-ice-*.txt)
    if [ "${#dumps[@]}" -eq 0 ]; then
        echo "没有找到 rustc-ice-*.txt。"
        echo "（rustc 把转储写在**当前工作目录**而不是 target/ 下；.gitignore 已经挡住它们，"
        echo "  所以 git status 干净不代表没有——用这条配方看，别看 git。）"
        exit 0
    fi
    channel=$(grep -m1 '^channel' rust-toolchain.toml 2>/dev/null | sed -E 's/.*"([^"]+)".*/\1/')
    echo "发现 ${#dumps[@]} 个 ICE 转储；rust-toolchain.toml 声明的 channel：${channel:-（读不到）}"
    echo ""
    for f in "${dumps[@]}"; do
        echo "── ${f}"
        # 第 1、2 行是 panic 位置与消息
        sed -n '1,2p' "$f" | sed 's/^/     /'
        grep -m1 '^rustc version:' "$f" | sed 's/^/     /'
        # query stack 只取前几层
        sed -n '/^query stack during panic:/,/^end of query stack/p' "$f" \
            | grep -E '^#[0-9]' | head -5 | sed 's/^/     /'
        echo ""
    done
    echo "接下来："
    echo "  1. 先核对上面的 rustc version 和 channel 是不是同一个编译器。"
    echo "     对不上 -> 你的 rust-toolchain.toml 没生效，先跑 just doctor。"
    echo "  2. 对得上 -> 就是这一版编译器在你的代码上崩了。照 query stack 找到那个"
    echo "     函数 / 类型，那里多半有个能换写法绕开的构造。"
    echo "  3. 要立刻恢复工作：把**你这个项目**的 channel 钉到前几天的 nightly ——"
    echo "     rust-toolchain.toml 里写 channel = \"nightly-YYYY-MM-DD\"。"
    echo "     这是项目级的临时措施，修好之后记得改回 \"nightly\" 或往前挪。"
    echo "  4. 值得上报：https://github.com/rust-lang/rust/issues （附完整转储文件）"
    echo ""
    echo "清理：rm -f rustc-ice-*.txt"

# 确认 Cargo.lock 与 Cargo.toml 对得上。
#
# 下面 lint / test 用的命令不带 --locked，依赖对不上时它们会顺手改写 Cargo.lock 再继续，
# 于是本地全绿、推上去 CI 却用提交里那份旧 lock 失败（CI 与 Dockerfile 全程 --locked）。
[private]
_lock-fresh: _generated-only
    #!/usr/bin/env bash
    set -euo pipefail
    if ! cargo metadata --locked --format-version 1 >/dev/null 2>&1; then
        {
            echo "✗ Cargo.lock 与 Cargo.toml 对不上（或还没生成）。"
            echo "  CI 与 Dockerfile 全程用 --locked，这样推上去会直接失败。"
            echo "  跑 just bootstrap 重新生成，并把 Cargo.lock 一起提交。"
        } >&2
        exit 1
    fi

# 覆盖 CI 里的 lint / test / deny 三个 job。
# 不含 hack / msrv / nll，它们各自是独立配方，CI 上照常会跑。
[group('check')]
[doc('本地跑一遍 CI 的主要检查（lint / test / audit）')]
ci: _lock-fresh lint test audit

# ---------------------------------------------------------------------------
# 依赖维护
# ---------------------------------------------------------------------------

[group('deps')]
[doc('按 Cargo.toml 的版本约束升级 Cargo.lock')]
update: _generated-only
    cargo update
    cargo deny check -A unmatched-bypass

[group('deps')]
[doc('列出可升级的依赖（需要 cargo-outdated）')]
outdated: _generated-only
    cargo outdated --root-deps-only --exit-code 1

[group('deps')]
[doc('更新 git submodule')]
update-submodule:
    git submodule update --init --recursive --remote

# ---------------------------------------------------------------------------
# 发布
# ---------------------------------------------------------------------------

[group('release')]
[doc('生成 / 更新 CHANGELOG.md')]
changelog:
    #!/usr/bin/env bash
    set -euo pipefail
    # --offline: 只用 owner/repo 拼链接，不去调平台 API
    just _cliff --offline -o CHANGELOG.md

# 内部配方：把 CHANGELOG 生成到指定版本，供 release.toml 的 pre-release-hook 调用
[private]
_changelog-for version:
    #!/usr/bin/env bash
    set -euo pipefail
    just _cliff --offline --tag "v{{ version }}" -o CHANGELOG.md

# 内部配方：带上正确的平台变量调用 git-cliff，两个 changelog 配方共用
[private]
_cliff *args:
    #!/usr/bin/env bash
    set -euo pipefail
    slug="{{ repo_slug }}"
    host="{{ repo_host }}"
    if [ -z "$slug" ] || [ -z "$host" ]; then
        echo "警告: 未识别到 github / gitlab 的 origin remote，CHANGELOG 里的提交链接会不完整" >&2
        git cliff {{ args }}
    elif [ "$host" = "gitlab" ]; then
        GITLAB_REPO="$slug" git cliff {{ args }}
    else
        GITHUB_REPO="$slug" git cliff {{ args }}
    fi

[group('release')]
[doc('发版预演：跑全套检查 + 干跑一遍，看清楚会改什么。level: patch|minor|major')]
release level="patch": ci
    # 先干跑一遍确认改动符合预期，再真正执行
    cargo release {{ level }}
    @echo ""
    @echo "以上是预演结果。确认无误后执行："
    @echo "    just release-execute {{ level }}"

# 这条不依赖 `ci`：正常流程是先 `just release` 预演（那一步已经跑过全套检查）。
# 单独用它发版时请自己先跑一次 `just ci`。
[group('release')]
[doc('真正执行发版（跳过预演；请确保刚跑过 just release 或 just ci）')]
release-execute level="patch": _generated-only
    cargo release {{ level }} --execute

# ---------------------------------------------------------------------------
# 初始化
# ---------------------------------------------------------------------------

# 刚 cargo generate 出来之后跑的第一条命令：生成 Cargo.lock（CI 与 Dockerfile
# 全程用 --locked，缺了它第一次推送就会失败）并启用 git 钩子。
[group('setup')]
[doc('首次拉起项目：生成 Cargo.lock、启用 git 钩子')]
bootstrap: _generated-only
    #!/usr/bin/env bash
    set -euo pipefail
    # cargo fetch 只解析依赖树并下载，不编译
    cargo fetch
    echo "✓ Cargo.lock 已就绪"
    just hooks
    echo ""
    echo "接下来：just doctor 体检工具链，just ci 走一遍完整检查。"

[group('setup')]
[doc('体检：检查工具链组件与配套 cargo 工具是否齐全，并给出补装命令')]
doctor:
    #!/usr/bin/env bash
    # 不加 `set -e`：体检要把所有问题一次列全
    set -uo pipefail
    missing=0

    echo "== 工具链 =="
    if ! command -v rustup >/dev/null 2>&1; then
        echo "  ✗ 未找到 rustup（https://rustup.rs）"
        exit 1
    fi
    channel=$(grep -m1 '^channel' rust-toolchain.toml | sed -E 's/.*"([^"]+)".*/\1/')
    echo "  rust-toolchain.toml 声明的 channel: ${channel}"
    rustc --version 2>/dev/null | sed 's/^/  /' || {
        echo "  ✗ 工具链 ${channel} 尚未安装 -> rustup toolchain install"
        missing=1
    }

    # 校验 rust-toolchain.toml 是否还说了算：rustup 的目录 override 和
    # RUSTUP_TOOLCHAIN 环境变量优先级都比它高，且完全静默。
    # `rustup show active-toolchain` 会把生效原因写在括号里。
    active=$(rustup show active-toolchain 2>/dev/null || true)
    if [ -n "$active" ]; then
        echo "  实际生效的工具链: ${active}"
        case "$active" in
            *"directory override"*)
                echo "  ✗ 存在 rustup 目录 override，rust-toolchain.toml 被架空 -> rustup override unset"
                missing=1 ;;
            *"environment variable RUSTUP_TOOLCHAIN"*)
                echo "  ✗ RUSTUP_TOOLCHAIN 环境变量覆盖了 rust-toolchain.toml -> unset RUSTUP_TOOLCHAIN"
                missing=1 ;;
            *rust-toolchain.toml*)
                echo "  ✓ 由 rust-toolchain.toml 决定" ;;
            *)
                # 多半是 "(default)"：rustup 没读到本项目的 toolchain 文件
                echo "  ✗ 不是由 rust-toolchain.toml 决定的 -> 确认在项目根目录下运行，且该文件还在"
                missing=1 ;;
        esac
    fi

    echo "== 组件 =="
    installed=$(rustup component list --installed 2>/dev/null)
    # llvm-tools 在 `component list` 里显示为 llvm-tools（不带 -preview 后缀）
    for c in clippy rust-src llvm-tools; do
        if grep -q "^${c}" <<<"$installed"; then
            echo "  ✓ ${c}"
        else
            echo "  ✗ ${c} -> rustup component add ${c}"
            missing=1
        fi
    done

    # 查的是 just fmt / just lint 真正会用的那条工具链（见文件开头的 fmt_toolchain）
    if rustup component list --toolchain '{{ fmt_toolchain }}' --installed 2>/dev/null | grep -q '^rustfmt'; then
        echo "  ✓ rustfmt ({{ fmt_toolchain }})"
    else
        echo "  ✗ rustfmt ({{ fmt_toolchain }}) -> just install-tools"
        missing=1
    fi

    echo "== 配套工具 =="
    for t in cargo-nextest cargo-deny cargo-llvm-cov cargo-release cargo-outdated \
             cargo-machete cargo-semver-checks cargo-hack typos taplo git-cliff bacon; do
        if command -v "$t" >/dev/null 2>&1; then
            echo "  ✓ ${t}"
        else
            echo "  ✗ ${t} -> just install-tools"
            missing=1
        fi
    done

    # ICE 转储只提示，不计入缺失项
    shopt -s nullglob
    ice_dumps=(rustc-ice-*.txt)
    if [ "${#ice_dumps[@]}" -gt 0 ]; then
        echo "== 编译器崩溃 =="
        echo "  ⚠️ 发现 ${#ice_dumps[@]} 个 rustc-ice-*.txt（编译器内部错误转储）-> just ice"
    fi

    echo "== git 钩子 =="
    if [ "$(git config --get core.hooksPath 2>/dev/null)" = ".githooks" ]; then
        echo "  ✓ .githooks 已启用（commit-msg / pre-push）"
    else
        echo "  - 未启用 -> just hooks"
    fi

    echo "== 可选 =="
    for t in cargo-binstall cargo-flamegraph docker; do
        command -v "$t" >/dev/null 2>&1 \
            && echo "  ✓ ${t}" \
            || echo "  - ${t}（未安装，非必需）"
    done

    echo ""
    if [ "$missing" -eq 0 ]; then
        echo "一切就绪，可以 just ci 了。"
    else
        echo "有缺失项，按上面的 -> 提示补装后重跑 just doctor。"
        exit 1
    fi

[group('setup')]
[doc('安装本项目用到的全部 cargo 工具')]
install-tools:
    #!/usr/bin/env bash
    set -euo pipefail
    tools=(
        cargo-nextest      # 测试运行器
        cargo-deny         # 依赖安全与 License 检查
        cargo-llvm-cov     # 覆盖率
        cargo-release      # 发版
        cargo-outdated     # 检查依赖是否有新版本
        cargo-machete      # 找出没用到的依赖
        cargo-semver-checks # 公开 API 的破坏性变更检查
        typos-cli          # 拼写检查
        taplo-cli          # TOML 格式化与检查
        git-cliff          # 生成 CHANGELOG
        bacon              # 后台实时监控
        cargo-hack         # feature 幂集检查，和 CI 的 hack job 对齐
    )
    # 优先用 cargo-binstall 下预编译二进制，没有预编译包的会自动退回源码编译
    if command -v cargo-binstall >/dev/null 2>&1; then
        cargo binstall --no-confirm --locked "${tools[@]}"
    else
        echo "提示：先装 cargo-binstall 能直接下预编译二进制，比源码编译快一个数量级："
        echo "        cargo install cargo-binstall"
        echo "      （其它安装方式见 https://github.com/cargo-bins/cargo-binstall）"
        echo "本次先用 cargo install 逐个编译，请耐心等待……"
        echo ""
        cargo install --locked "${tools[@]}"
    fi
    # channel 本身是 nightly 时，rust-toolchain.toml 的 components 里已经带了 rustfmt，
    # 只有非 nightly 才需要额外装一份。
    channel=$(grep -m1 '^channel' rust-toolchain.toml 2>/dev/null | sed -E 's/.*"([^"]+)".*/\1/')
    if [ "${channel#nightly}" = "$channel" ]; then
        # --allow-downgrade：当天 nightly 缺 rustfmt 组件时自动退回最近一个齐全的版本
        rustup toolchain install nightly --allow-downgrade --profile minimal --component rustfmt
    fi

# 用 core.hooksPath 启用仓库里的 .githooks/，每个 clone 都要跑一次。
[group('setup')]
[doc('启用 git 钩子（commit-msg 校验提交信息 / pre-push 跑 just ci）')]
hooks:
    #!/usr/bin/env bash
    set -euo pipefail
    # 清理早期版本用 pre-commit 装进 .git/hooks/ 的脚本：它们写死了已不存在的
    # --config=.pre-commit-config.yaml，会在每次 commit 时报错。
    # 只删自报家门的那些（文件头有 pre-commit 的生成标记），手写的钩子不动。
    for h in pre-commit commit-msg pre-push post-commit post-checkout post-merge; do
        f=".git/hooks/$h"
        if [ -f "$f" ] && grep -q "File generated by pre-commit" "$f" 2>/dev/null; then
            rm -f "$f"
            echo "  已清理遗留的 pre-commit 钩子：$f"
            # pre-commit 安装时会把原有的同名钩子改名备份成 .legacy
            if [ -f "$f.legacy" ]; then
                echo "  ⚠️ 发现 $f.legacy（pre-commit 当初备份的旧钩子），保留着，需要的话自己看一眼"
            fi
        fi
    done
    # cargo-generate 不保证保留可执行位，这里补一次
    chmod +x .githooks/*
    git config core.hooksPath .githooks
    echo "✓ 已启用 .githooks/"
    echo "    pre-commit  按改动跑快速检查（fmt / clippy / taplo / typos / 私钥）"
    echo "    commit-msg  校验 Conventional Commits（CHANGELOG 与版本推导依赖它）"
    echo "    pre-push    跑一遍 just ci（lint / test / audit）"
    echo "  临时跳过：git commit --no-verify / git push --no-verify"
    echo "  停用：git config --unset core.hooksPath"
