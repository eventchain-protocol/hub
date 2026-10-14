#ifndef EVENTCHAIN_ACCEPTOR_H
#define EVENTCHAIN_ACCEPTOR_H

#include <stddef.h>
#include <stdint.h>

/* Start and stop the GHC runtime. Call ec_init once before any other
 * function and ec_exit once at process end. */
void ec_init(void);
void ec_exit(void);

/* Status codes shared by every function below. */
enum ec_status {
    EC_OK = 0,
    EC_KEY_REJECTED = 1,   /* not a 32-byte scalar in [1, n) */
    EC_CRYPTO_FAILED = 2   /* libcrypto did not answer */
};

/* The compressed public point for a 32-byte private scalar.
 * out must hold 33 bytes. */
int32_t ec_public_key(const uint8_t *key, size_t key_len, uint8_t *out);

/* Deterministic ECDSA-P256 (RFC 6979) over msg with the 32-byte scalar key,
 * as eventchain-crypto's `sign` computes it. out must hold 64 bytes: r || s. */
int32_t ec_sign(const uint8_t *key, size_t key_len,
                const uint8_t *msg, size_t msg_len,
                uint8_t *out);

#endif
