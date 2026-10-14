//! Safe wrapper over the C ABI in `acceptor/include/eventchain_acceptor.h`.
//! Judging is eventchain-verify's and signing eventchain-crypto's; this
//! module only moves bytes across and names what comes back.

use std::ffi::c_int;

/// The opaque key handle the library hands out.
#[repr(C)]
pub struct EcKey {
    _private: [u8; 0],
}

#[link(name = "eventchain_acceptor")]
extern "C" {
    fn ec_init();
    fn ec_exit();
    fn ec_key_load(scalar: *const u8, scalar_len: usize, out: *mut *mut EcKey) -> c_int;
    fn ec_key_free(key: *mut EcKey);
    fn ec_key_public(key: *const EcKey, out: *mut u8);
    fn ec_sign(key: *const EcKey, msg: *const u8, msg_len: usize, out: *mut u8) -> c_int;
    fn ec_accept(
        count: u64,
        head: *const u8,
        line: *const u8,
        line_len: usize,
        out_verdict: *mut i32,
        out_fault: *mut i32,
        out_hash: *mut u8,
    ) -> c_int;
}

// The payloads are read through Debug when an error is reported.
#[allow(dead_code)]
#[derive(Debug)]
pub enum Error {
    KeyRejected,
    CryptoFailed,
    BadArgument,
    NotOneLine,
    UnknownStatus(c_int),
    UnknownVerdict(i32),
    UnknownFault(i32),
}

fn status(code: c_int) -> Result<(), Error> {
    match code {
        0 => Ok(()),
        1 => Err(Error::KeyRejected),
        2 => Err(Error::CryptoFailed),
        3 => Err(Error::BadArgument),
        4 => Err(Error::NotOneLine),
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

/// The Hub key, loaded once. The scalar lives on the Haskell side.
pub struct Key(*mut EcKey);

// The handle is a GHC stable pointer to an immutable value; any thread may use it.
unsafe impl Send for Key {}
unsafe impl Sync for Key {}

impl Key {
    pub fn load(scalar: [u8; 32]) -> Result<Key, Error> {
        let mut handle: *mut EcKey = std::ptr::null_mut();
        status(unsafe { ec_key_load(scalar.as_ptr(), scalar.len(), &mut handle) })?;
        Ok(Key(handle))
    }

    pub fn public_key(&self) -> [u8; 33] {
        let mut out = [0u8; 33];
        unsafe { ec_key_public(self.0, out.as_mut_ptr()) };
        out
    }

    /// Deterministic ECDSA-P256 over exactly these bytes.
    pub fn sign(&self, message: &[u8]) -> Result<[u8; 64], Error> {
        let mut out = [0u8; 64];
        status(unsafe { ec_sign(self.0, message.as_ptr(), message.len(), out.as_mut_ptr()) })?;
        Ok(out)
    }
}

impl Drop for Key {
    fn drop(&mut self) {
        unsafe { ec_key_free(self.0) };
    }
}

/// The head of an empty topic: 32 zero bytes.
#[allow(dead_code)]
pub const GENESIS: [u8; 32] = [0u8; 32];

/// What the verifier said about one line.
#[allow(dead_code)]
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// An Entry; every check held. The hash is the Event ID.
    Sound { event_id: [u8; 32] },
    /// Not an Entry at all.
    Malformed(Fault),
    /// An Entry; at least one check said no.
    Unsound(Fault),
    /// libcrypto did not answer. Says nothing about the line.
    Undecided(Fault),
}

/// Judge `line`, the exact bytes of one line without its terminator, as line
/// number `count` of a topic whose last line hashes to `head`.
#[allow(dead_code)]
pub fn accept(count: u64, head: &[u8; 32], line: &[u8]) -> Result<Verdict, Error> {
    let mut verdict: i32 = -1;
    let mut fault: i32 = -1;
    let mut hash = [0u8; 32];
    status(unsafe {
        ec_accept(count, head.as_ptr(), line.as_ptr(), line.len(), &mut verdict, &mut fault, hash.as_mut_ptr())
    })?;
    match verdict {
        0 => Ok(Verdict::Sound { event_id: hash }),
        1 => Ok(Verdict::Malformed(Fault::from_code(fault)?)),
        2 => Ok(Verdict::Unsound(Fault::from_code(fault)?)),
        3 => Ok(Verdict::Undecided(Fault::from_code(fault)?)),
        other => Err(Error::UnknownVerdict(other)),
    }
}

/// The closed fault vocabulary of `enum ec_fault` in the header, with the
/// name a refusal's `reason` carries. Numbers and names are the contract.
macro_rules! faults {
    ($($variant:ident = $code:literal => $name:literal,)*) => {
        #[allow(dead_code)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Fault {
            $($variant = $code,)*
        }

        impl Fault {
            fn from_code(code: i32) -> Result<Fault, Error> {
                match code {
                    $($code => Ok(Fault::$variant),)*
                    other => Err(Error::UnknownFault(other)),
                }
            }

            #[allow(dead_code)]
            pub fn name(self) -> &'static str {
                match self {
                    $(Fault::$variant => $name,)*
                }
            }
        }
    };
}

faults! {
    ChainMismatch = 1 => "chain_mismatch",
    KeyOffCurve = 2 => "key_off_curve",
    SignatureInvalid = 3 => "signature_invalid",
    ClientDataNotUtf8 = 10 => "client_data_not_utf8",
    ClientDataNotJson = 11 => "client_data_not_json",
    ClientDataNotAnObject = 12 => "client_data_not_an_object",
    ClientDataDuplicate = 13 => "client_data_duplicate",
    ClientDataNotText = 14 => "client_data_not_text",
    ClientDataIncomplete = 15 => "client_data_incomplete",
    NotAnAssertion = 16 => "not_an_assertion",
    ChallengeMismatch = 17 => "challenge_mismatch",
    AuthenticatorDataTruncated = 18 => "authenticator_data_truncated",
    UserPresenceUnset = 19 => "user_presence_unset",
    BackupStateInconsistent = 20 => "backup_state_inconsistent",
    AssertionInvalid = 21 => "assertion_invalid",
    AttesterKeyOffCurve = 22 => "attester_key_off_curve",
    LineNotWellFormed = 100 => "line_not_well_formed",
    LineNotJson = 101 => "line_not_json",
    LineNotAnObject = 102 => "line_not_an_object",
    LineTrailingContent = 103 => "line_trailing_content",
    UnknownMember = 104 => "unknown_member",
    DuplicateMember = 105 => "duplicate_member",
    NonStringValue = 106 => "non_string_value",
    MissingMember = 107 => "missing_member",
    UnknownRevision = 108 => "unknown_revision",
    MemberOutsideRevision = 109 => "member_outside_revision",
    MemberOutsideKind = 110 => "member_outside_kind",
    UnknownKind = 111 => "unknown_kind",
    NotBase64Url = 112 => "not_base64url",
    MemberWrongShape = 113 => "member_wrong_shape",
    FileHasBom = 114 => "file_has_bom",
    CryptoKeyNotOnCurve = 200 => "crypto_key_not_on_curve",
    CryptoScalarWrongLength = 201 => "crypto_scalar_wrong_length",
    CryptoScalarRejected = 202 => "crypto_scalar_rejected",
    CryptoSigOutOfRange = 203 => "crypto_sig_out_of_range",
    CryptoOpensslFailed = 204 => "crypto_openssl_failed",
}
