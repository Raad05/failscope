//! Yellowstone gRPC ingest of failed transactions: subscribe, convert,
//! decode, store, with reconnect, resume and gap tracking.

mod convert;
mod pipeline;

pub use convert::{tx_input, ConvertError};
pub use pipeline::{decode_with_idls, run, IngestConfig, IngestError, IngestStats};
pub use yellowstone_grpc_proto;
