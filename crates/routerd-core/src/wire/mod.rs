//! Syntrop Cluster Mesh Protocol (SCMP) wire framing and transport.

pub mod framing;

pub use framing::{
    configure_mesh_tcp, ScmpFrame, ScmpHeader, ScmpMessageType, HEADER_LEN, SCMP_MAGIC,
    SCMP_VERSION,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_magic_constant() {
        assert_eq!(SCMP_MAGIC, b"SCMP");
        assert_eq!(SCMP_VERSION, 1);
    }
}
