//! Where a worker listens and how it is told: a per-launch token and a private local address. Never TCP.
//! Unix: a socket inside a freshly created directory with mode 0700. Windows: a randomly named pipe that only
//! the current user can open. The implementation is shared with the plugin host (`imt_ipc::endpoint`).

pub use imt_ipc::endpoint::{new_token, random_hex, tokens_equal, Handshake, PrivateEndpoint};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_random_and_compared_exactly() {
        let a = new_token().unwrap();
        assert_eq!(a.len(), 64);
        assert_ne!(a, new_token().unwrap());
        assert!(tokens_equal(&a, &a.clone()));
        assert!(!tokens_equal(&a, &a[..63]) && !tokens_equal(&a, "") && !tokens_equal("", &a));
        assert!(tokens_equal("", ""));
        assert!(!tokens_equal("abc", "abd"));
        assert_eq!(random_hex(3).unwrap().len(), 6);
    }

    #[cfg(unix)]
    #[test]
    fn the_socket_directory_is_private_and_removed_on_drop() {
        use std::os::unix::fs::PermissionsExt;
        let e = PrivateEndpoint::create().unwrap();
        let dir = std::path::Path::new(&e.address).parent().unwrap().to_path_buf();
        assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
        assert!(e.address.ends_with("w.sock") && e.address.len() < 100);
        let other = PrivateEndpoint::create().unwrap();
        assert_ne!(e.address, other.address);
        drop(e);
        assert!(!dir.exists());
    }

    #[test]
    fn the_handshake_roundtrips() {
        let h = Handshake {
            address: "a".into(),
            token: "t".into(),
        };
        assert_eq!(serde_json::from_str::<Handshake>(&serde_json::to_string(&h).unwrap()).unwrap(), h);
    }
}
