//! Who may do what. The paper makes payload access application policy and
//! has the Hub enforce it without holding decryption keys, so the handlers
//! ask an `AccessPolicy` and never look at a credential themselves. The one
//! implementation is deliberately small: per-topic bearer tokens from
//! `topic.json`, each mapped to a role that may read all of the topic's
//! Payloads, or read and write them, and nothing finer. It shows the Hub
//! enforcing a policy and is not an authorisation system.
//!
//! The operator token that gates topic creation sits here too. It is not a
//! topic's policy, since the topic does not exist yet, but it is the same
//! question asked of the same kind of credential.
//!
//! Tokens are compared as SHA-256 digests in constant time, so neither the
//! content nor the length of a stored token shows in how long a refusal takes.

use serde::Deserialize;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// What a request asks to do on a topic.
#[derive(Clone, Copy)]
pub enum Action {
    ReadPayload,
    WritePayload,
    Subscribe,
}

/// Why a request may not do it.
#[derive(Debug)]
pub enum Denied {
    /// No credential, or one the policy does not know: a 401.
    Unauthenticated,
    /// A known credential without the right: a 403.
    Forbidden,
}

pub trait AccessPolicy: Send + Sync {
    /// Whether the holder of `bearer`, the token from `Authorization: Bearer`, may do `action`.
    fn authorise(&self, bearer: Option<&str>, action: Action) -> Result<(), Denied>;
}

#[derive(Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// May get every Payload of the topic.
    Read,
    /// May put and get every Payload of the topic, and subscribe to it.
    Write,
}

/// One member of `topic.json`'s `access` array.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    pub token: String,
    pub role: Role,
}

/// The token implementation: the topic's grants, held as digests.
pub struct TopicTokens {
    grants: Vec<([u8; 32], Role)>,
}

impl TopicTokens {
    /// Every token is one a client can send and none appears twice.
    pub fn new(grants: &[Grant]) -> Result<TopicTokens, String> {
        let mut digests: Vec<([u8; 32], Role)> = Vec::with_capacity(grants.len());
        for (number, grant) in grants.iter().enumerate() {
            check_token(&grant.token).map_err(|e| format!("access[{number}]: {e}"))?;
            let digest = digest(&grant.token);
            if digests.iter().any(|(known, _)| *known == digest) {
                return Err(format!("access[{number}]: the token is granted twice"));
            }
            digests.push((digest, grant.role));
        }
        Ok(TopicTokens { grants: digests })
    }

    /// The role of the presented token. Every grant is compared, whichever matches.
    fn role(&self, bearer: &str) -> Option<Role> {
        let presented = digest(bearer);
        let mut role = None;
        for (known, granted) in &self.grants {
            if bool::from(known.ct_eq(&presented)) {
                role = Some(*granted);
            }
        }
        role
    }
}

impl AccessPolicy for TopicTokens {
    fn authorise(&self, bearer: Option<&str>, action: Action) -> Result<(), Denied> {
        let role = bearer.and_then(|token| self.role(token)).ok_or(Denied::Unauthenticated)?;
        match (action, role) {
            (Action::ReadPayload, _) | (_, Role::Write) => Ok(()),
            (Action::WritePayload | Action::Subscribe, Role::Read) => Err(Denied::Forbidden),
        }
    }
}

/// The operator's token, from `HUB_ADMIN_TOKEN`. Only it may create a topic.
pub struct Operator {
    digest: [u8; 32],
}

impl Operator {
    pub fn new(token: &str) -> Result<Operator, String> {
        check_token(token)?;
        Ok(Operator { digest: digest(token) })
    }

    /// Any token other than the operator's is a 403, none a 401.
    pub fn authorise(&self, bearer: Option<&str>) -> Result<(), Denied> {
        let token = bearer.ok_or(Denied::Unauthenticated)?;
        if bool::from(self.digest.ct_eq(&digest(token))) {
            Ok(())
        } else {
            Err(Denied::Forbidden)
        }
    }
}

/// A token travels after `Bearer ` in a header, so it is visible ASCII and not empty.
fn check_token(token: &str) -> Result<(), String> {
    if token.is_empty() || !token.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
        return Err("a token is one or more visible ASCII characters".to_string());
    }
    Ok(())
}

fn digest(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}
