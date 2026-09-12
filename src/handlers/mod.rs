mod auth;
mod chat;
mod message;

pub use crate::handlers::auth::AuthOutput;
pub(crate) use crate::handlers::{
    auth::{me, signin, signup},
    chat::{create_chat, list_chats},
    message::{list_messages, send_message},
};
