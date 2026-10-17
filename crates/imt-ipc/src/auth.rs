//! The per-launch token: the client attaches it to every call, the server refuses calls without it.

use crate::endpoint::tokens_equal;
use tonic::metadata::{Ascii, MetadataValue};
use tonic::service::Interceptor;
use tonic::{Request, Status};

/// Metadata key carrying the per-launch token on every call.
pub const TOKEN_HEADER: &str = "x-imt-token";

/// Client side: adds the token to every request.
#[derive(Clone)]
pub struct AttachToken(MetadataValue<Ascii>);

impl AttachToken {
    /// Fails when the token is not valid header text.
    pub fn new(token: &str) -> Result<AttachToken, crate::IpcError> {
        token
            .parse()
            .map(AttachToken)
            .map_err(|_| crate::IpcError::Protocol("the session token is not valid".into()))
    }
}

impl Interceptor for AttachToken {
    fn call(&mut self, mut req: Request<()>) -> Result<Request<()>, Status> {
        req.metadata_mut().insert(TOKEN_HEADER, self.0.clone());
        Ok(req)
    }
}

/// Server side: lets a request through only with the expected token (compared in constant time).
#[derive(Clone)]
pub struct RequireToken(std::sync::Arc<str>);

impl RequireToken {
    pub fn new(token: &str) -> RequireToken {
        RequireToken(token.into())
    }
}

impl Interceptor for RequireToken {
    fn call(&mut self, req: Request<()>) -> Result<Request<()>, Status> {
        match req.metadata().get(TOKEN_HEADER).and_then(|v| v.to_str().ok()) {
            Some(t) if tokens_equal(t, &self.0) => Ok(req),
            _ => Err(Status::unauthenticated("missing or wrong session token")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_token_is_attached_and_checked() {
        let mut attach = AttachToken::new("abc").unwrap();
        let req = attach.call(Request::new(())).unwrap();
        assert!(RequireToken::new("abc").call(req).is_ok());
        let req = AttachToken::new("abd").unwrap().call(Request::new(())).unwrap();
        assert_eq!(RequireToken::new("abc").call(req).unwrap_err().code(), tonic::Code::Unauthenticated);
        assert!(RequireToken::new("abc").call(Request::new(())).is_err());
        assert!(AttachToken::new("bad\ntoken").is_err());
    }
}
