//! The HTTP surface, one module per resource.

pub mod account;
pub mod dead_letter;
pub mod health;
pub mod leverage;
pub mod plan;
pub mod price;
pub mod quota;
pub mod session;
pub mod source;
pub mod usage;

use poem_openapi::Object;

#[derive(Debug, Object)]
pub struct Error {
    pub message: String,
}
