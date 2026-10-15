#![doc = include_str!("../README.md")]

pub mod browser;
mod connection;
mod error;
mod ice;
mod sdp;
mod stream;
mod transport;
mod upgrade;

pub use self::{
    browser::{Config as BrowserConfig, Transport as BrowserTransport},
    connection::Connection,
    error::{Error, IceCheckOutcome},
    ice::{IceServer, IceServers},
    stream::Stream,
    transport::{Config, Transport},
};
