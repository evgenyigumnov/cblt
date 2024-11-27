use http::StatusCode;
use humantime::DurationError;
use rustls::pki_types;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CbltError {
    #[error("ParseRequestError: {details:?}")]
    RequestError {
        details: String,
        status_code: StatusCode,
    },
    #[error("DirectiveNotMatched")]
    DirectiveNotMatched,
    #[error("ResponseError: {status_code:?} {details:?}")]
    ResponseError {
        details: String,
        status_code: StatusCode,
    },
    #[error("IOError: {source:?}")]
    IOError {
        #[from]
        source: std::io::Error,
    },
    // from AcquireError
    #[error("AcquireError: {source:?}")]
    AcquireError {
        #[from]
        source: tokio::sync::AcquireError,
    },
    // from rustls::Error
    #[error("RustlsError: {source:?}")]
    RustlsError {
        #[from]
        source: rustls::Error,
    },
    // from bollard::errors::Error
    #[error("BollardError: {source:?}")]
    BollardError {
        #[from]
        source: bollard::errors::Error,
    },

    // from pki_types::pem::Error
    #[error("PemError: {source:?}")]
    PemError {
        #[from]
        source: pki_types::pem::Error,
    },
    // from http::Error
    #[error("HttpError: {source:?}")]
    HttpError {
        #[from]
        source: http::Error,
    },
    // from http::header::ToStrError
    #[error("ToStrError: {source:?}")]
    ToStrError {
        #[from]
        source: http::header::ToStrError,
    },
    // from KdlError
    #[error("KdlError: {source:?}")]
    KdlError {
        #[from]
        source: kdl::KdlError,
    },
    // from SystemTimeError
    #[error("SystemTimeError: {source:?}")]
    SystemTimeError {
        #[from]
        source: std::time::SystemTimeError,
    },
    // from std::num::ParseIntError
    #[error("ParseIntError: {source:?}")]
    ParseIntError {
        #[from]
        source: std::num::ParseIntError,
    },
    // from DurationError
    #[error("DurationError: {source:?}")]
    DurationError {
        #[from]
        source: DurationError,
    },

    #[error("KdlParseError: {details:?}")]
    KdlParseError { details: String },
    #[error("HeaplessError")]
    HeaplessError,
    #[error("ServiceNameNotFound")]
    ServiceNameNotFound,
    #[error("ContainerNameNotFound")]
    ContainerNameNotFound,
}
