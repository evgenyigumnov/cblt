use http::StatusCode;
use rustls::pki_types;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CbltError {
    #[error("ParseRequestError: {details:?}")]
    ParseRequestError { details: String },
    #[error("RequestError: {status_code:?} {details:?}")]
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
    // from reqwest::Error
    #[error("ReqwestError: {source:?}")]
    ReqwestError {
        #[from]
        source: reqwest::Error,
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
    #[error("AbsentKey")]
    AbsentKey,
    #[error("AbsentCert")]
    AbsentCert,
    #[error("KdlParseError: {details:?}")]
    KdlParseError { details: String },
}
