#ifndef EVENTCHAIN_ACCEPTOR_H
#define EVENTCHAIN_ACCEPTOR_H

#include <stddef.h>
#include <stdint.h>

/* The contract between the Hub and the eventchain libraries. A change here
 * is a breaking change on both sides; nothing shims across versions. */

/* Start and stop the GHC runtime. Call ec_init once before any other
 * function and ec_exit once at process end. */
void ec_init(void);
void ec_exit(void);

/* Status codes shared by every function that returns int32_t. */
enum ec_status {
    EC_OK = 0,
    EC_KEY_REJECTED = 1,   /* not a 32-byte scalar in [1, n) */
    EC_CRYPTO_FAILED = 2,  /* libcrypto did not answer */
    EC_BAD_ARGUMENT = 3,   /* count is zero and head is not 32 zero bytes */
    EC_NOT_ONE_LINE = 4    /* the bytes framed as more or fewer than one line */
};

/* The Hub key, loaded once. Opaque: the scalar never comes back. */
typedef struct ec_key ec_key;

/* Load a 32-byte P-256 private scalar. On EC_OK *out holds the handle,
 * released with ec_key_free. */
int32_t ec_key_load(const uint8_t *scalar, size_t scalar_len, ec_key **out);
void ec_key_free(ec_key *key);

/* The compressed public point. out must hold 33 bytes. */
void ec_key_public(const ec_key *key, uint8_t *out);

/* Deterministic ECDSA-P256 (RFC 6979) over msg, as eventchain-crypto's
 * `sign` computes it. The caller supplies the exact bytes to sign; for a
 * heartbeat, receipt or refusal that is the JCS serialisation of the
 * statement without its signature member. out must hold 64 bytes: r || s. */
int32_t ec_sign(const ec_key *key, const uint8_t *msg, size_t msg_len, uint8_t *out);

/* What the Verifier said about one line. */
enum ec_verdict {
    EC_SOUND = 0,       /* an Entry; every check held */
    EC_MALFORMED = 1,   /* not an Entry at all */
    EC_UNSOUND = 2,     /* an Entry; at least one check said no */
    EC_UNDECIDED = 3    /* libcrypto did not answer; says nothing about the line */
};

/* Why, when the verdict is not EC_SOUND. One closed vocabulary: the
 * Verifier's own fault and error constructors, numbered by verdict.
 * A refusal's `reason` is the snake_case name of one of these. */
enum ec_fault {
    EC_FAULT_NONE = 0,

    /* EC_UNSOUND: the Verifier's Fault. The first fault when there are several. */
    EC_FAULT_CHAIN_MISMATCH = 1,
    EC_FAULT_KEY_OFF_CURVE = 2,
    EC_FAULT_SIGNATURE_INVALID = 3,
    /* EC_UNSOUND, a Mint envelope: the Verifier's EnvelopeFault. */
    EC_FAULT_CLIENT_DATA_NOT_UTF8 = 10,
    EC_FAULT_CLIENT_DATA_NOT_JSON = 11,
    EC_FAULT_CLIENT_DATA_NOT_AN_OBJECT = 12,
    EC_FAULT_CLIENT_DATA_DUPLICATE = 13,
    EC_FAULT_CLIENT_DATA_NOT_TEXT = 14,
    EC_FAULT_CLIENT_DATA_INCOMPLETE = 15,
    EC_FAULT_NOT_AN_ASSERTION = 16,
    EC_FAULT_CHALLENGE_MISMATCH = 17,
    EC_FAULT_AUTHENTICATOR_DATA_TRUNCATED = 18,
    EC_FAULT_USER_PRESENCE_UNSET = 19,
    EC_FAULT_BACKUP_STATE_INCONSISTENT = 20,
    EC_FAULT_ASSERTION_INVALID = 21,
    EC_FAULT_ATTESTER_KEY_OFF_CURVE = 22,

    /* EC_MALFORMED: the Verifier's LineError, plus the file-level BOM rule. */
    EC_FAULT_LINE_NOT_WELL_FORMED = 100,
    EC_FAULT_LINE_NOT_JSON = 101,
    EC_FAULT_LINE_NOT_AN_OBJECT = 102,
    EC_FAULT_LINE_TRAILING_CONTENT = 103,
    EC_FAULT_UNKNOWN_MEMBER = 104,
    EC_FAULT_DUPLICATE_MEMBER = 105,
    EC_FAULT_NON_STRING_VALUE = 106,
    EC_FAULT_MISSING_MEMBER = 107,
    EC_FAULT_UNKNOWN_REVISION = 108,
    EC_FAULT_MEMBER_OUTSIDE_REVISION = 109,
    EC_FAULT_MEMBER_OUTSIDE_KIND = 110,
    EC_FAULT_UNKNOWN_KIND = 111,
    EC_FAULT_NOT_BASE64URL = 112,
    EC_FAULT_MEMBER_WRONG_SHAPE = 113,
    EC_FAULT_FILE_HAS_BOM = 114,

    /* EC_UNDECIDED: eventchain-crypto's CryptoError. */
    EC_FAULT_CRYPTO_KEY_NOT_ON_CURVE = 200,
    EC_FAULT_CRYPTO_SCALAR_WRONG_LENGTH = 201,
    EC_FAULT_CRYPTO_SCALAR_REJECTED = 202,
    EC_FAULT_CRYPTO_SIG_OUT_OF_RANGE = 203,
    EC_FAULT_CRYPTO_OPENSSL_FAILED = 204
};

/* Judge one line as the next line of a topic.
 *
 * count is the number of lines the topic holds and head the SHA-256 of the
 * last one, 32 bytes; with count zero, head must be 32 zero bytes and the
 * line is judged as a file's opening bytes. line is the exact bytes of one
 * line without its terminator.
 *
 * On EC_OK: *out_verdict is an ec_verdict; *out_fault is an ec_fault, zero
 * when Sound; out_hash receives 32 bytes, the line's SHA-256 (the Event ID)
 * when Sound and zeros otherwise. */
int32_t ec_accept(uint64_t count, const uint8_t *head,
                  const uint8_t *line, size_t line_len,
                  int32_t *out_verdict, int32_t *out_fault, uint8_t *out_hash);

#endif
