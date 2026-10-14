//! Safe wrapper over the C ABI in `acceptor/include/eventchain_acceptor.h`.
//! Signing is eventchain-crypto's; this module only moves bytes across.

use std::ffi::c_int;

#[link(name = "eventchain_acceptor")]
extern "C" {
    fn ec_init();
    fn ec_exit();
    fn ec_public_key(key: *const u8, key_len: usize, out: *mut u8) -> c_int;
    fn ec_sign(key: *const u8, key_len: usize, msg: *const u8, msg_len: usize, out: *mut u8) -> c_int;
}

// The payloads are read through Debug when an error is reported.
#[allow(dead_code)]
#[derive(Debug)]
pub enum Error {
    KeyRejected,
    CryptoFailed,
    UnknownStatus(c_int),
}

fn status(code: c_int) -> Result<(), Error> {
    match code {
        0 => Ok(()),
        1 => Err(Error::KeyRejected),
        2 => Err(Error::CryptoFailed),
        other => Err(Error::UnknownStatus(other)),
    }
}

/// The GHC runtime, started once per process and stopped when dropped.
pub struct Runtime;

impl Runtime {
    pub fn start() -> Runtime {
        unsafe { ec_init() };
        Runtime
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        unsafe { ec_exit() };
    }
}

/// A 32-byte P-256 private scalar, checked by eventchain-crypto at use.
pub struct Key([u8; 32]);

impl Key {
    pub fn from_bytes(bytes: [u8; 32]) -> Key {
        Key(bytes)
    }

    pub fn public_key(&self) -> Result<[u8; 33], Error> {
        let mut out = [0u8; 33];
        status(unsafe { ec_public_key(self.0.as_ptr(), self.0.len(), out.as_mut_ptr()) })?;
        Ok(out)
    }

    pub fn sign(&self, message: &[u8]) -> Result<[u8; 64], Error> {
        let mut out = [0u8; 64];
        status(unsafe {
            ec_sign(self.0.as_ptr(), self.0.len(), message.as_ptr(), message.len(), out.as_mut_ptr())
        })?;
        Ok(out)
    }
}
