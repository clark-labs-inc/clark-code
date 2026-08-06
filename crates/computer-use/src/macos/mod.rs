pub(crate) mod auth;
pub(crate) mod client;
pub(crate) mod protocol;

#[cfg(feature = "helper-service")]
mod accessibility;
#[cfg(feature = "helper-service")]
mod capture;
#[cfg(feature = "helper-service")]
pub(crate) mod helper;
#[cfg(feature = "helper-service")]
mod input;
#[cfg(feature = "helper-service")]
mod permissions;
#[cfg(feature = "helper-service")]
mod service;
#[cfg(feature = "helper-service")]
mod windows;
