{- | The acceptor: one submitted line judged as the next line of a topic, by
the reference Verifier, with the verdict flattened to the codes that
@include/eventchain_acceptor.h@ declares.

The Hub holds what a resuming holder holds, the count of lines in the topic
and the digest of the last one, and the Verifier's fold continues from
there. Before the first line there is no checkpoint, so the submission is
judged as a file's opening bytes, which is also the only place the byte
order mark rule can apply. The checks are the Verifier's own: well formed
under the exact-bytes rules, @prev_hash@ equal to the head, signature valid
under the key the line carries, and for a Mint the envelope checks. Nothing
is re-implemented here; this module frames one line, calls the fold, and
names what came back.

The codes are the ABI contract. They are the closed vocabulary a refusal's
@reason@ draws from, so every constructor the Verifier can report has a
number here and in the header, and a constructor added upstream fails this
module's compile rather than mapping to something approximate.
-}
module EventChain.Acceptor
    ( Judgement (..)
    , AcceptError (..)
    , accept
    , Coded (..)
    , code
    ) where

import Data.ByteString (ByteString)
import Data.ByteString qualified as BS
import Data.ByteString.Lazy qualified as LBS
import Data.Int (Int32)
import Data.List.NonEmpty qualified as NE
import Data.Word (Word64)
import EventChain.Crypto (CryptoError (..))
import EventChain.Crypto.Types (LineHash, lineHashRaw, sha256Length)
import EventChain.Verify (Fault (..), Verdict (..), checkpointFrom, genesisHash, verifiedLineHash, verify, verifyFrom)
import EventChain.Verify.WebAuthn (EnvelopeFault (..))
import EventChain.Verify.Wire (DecodeError (..), FramingError (..), LineError (..))

-- | What the Verifier said about the line.
data Judgement
    = -- | The fold ran and gave this verdict.
      Judged Verdict
    | -- | The bytes could not open a file: only possible before the first line.
      Unframed FramingError
    deriving stock (Show)

-- | A call that could not be answered with a verdict at all.
data AcceptError
    = -- | The count is zero and the head is not the genesis sentinel.
      HeadNotGenesis
    | -- | The bytes framed as this many lines rather than one.
      NotOneLine Int
    deriving stock (Eq, Show)

{- | Judge @line@ as line number @count@ of a topic whose last line hashes to
@headHash@. @line@ is the exact bytes of one line without its terminator.
-}
accept :: Word64 -> LineHash -> ByteString -> Either AcceptError Judgement
accept count headHash line
    | count == 0 && headHash /= genesisHash = Left HeadNotGenesis
    | count == 0 = case verify bytes of
        Left framing -> Right (Unframed framing)
        Right verdicts -> one verdicts
    | otherwise = one (verifyFrom (checkpointFrom (count - 1) headHash) bytes)
  where
    bytes = LBS.fromStrict (line <> BS.singleton 0x0a)
    one [verdict] = Right (Judged verdict)
    one verdicts = Left (NotOneLine (length verdicts))

{- | A judgement as the header declares it: the verdict code, the fault code
(zero on Sound), and the line hash (32 zero bytes unless Sound).
-}
data Coded = Coded Int32 Int32 ByteString
    deriving stock (Eq, Show)

code :: Judgement -> Coded
code = \case
    Judged (Sound entry) -> Coded 0 0 (lineHashRaw (verifiedLineHash entry))
    Judged (Malformed (DecodeError _ reason)) -> Coded 1 (lineErrorCode reason) noHash
    Judged (Unsound _ faults) -> Coded 2 (faultCode (NE.head faults)) noHash
    Judged (Undecided _ err) -> Coded 3 (cryptoErrorCode err) noHash
    Unframed FileHasBom -> Coded 1 114 noHash
  where
    noHash = BS.replicate sha256Length 0

-- | An Unsound line's first fault.
faultCode :: Fault -> Int32
faultCode = \case
    ChainMismatch{} -> 1
    KeyOffCurve -> 2
    SignatureInvalid -> 3
    AttestationFault envelope -> envelopeFaultCode envelope

envelopeFaultCode :: EnvelopeFault -> Int32
envelopeFaultCode = \case
    ClientDataNotUtf8 -> 10
    ClientDataNotJson _ -> 11
    ClientDataNotAnObject -> 12
    ClientDataDuplicate _ -> 13
    ClientDataNotText _ -> 14
    ClientDataIncomplete _ -> 15
    NotAnAssertion _ -> 16
    ChallengeMismatch{} -> 17
    AuthenticatorDataTruncated _ -> 18
    UserPresenceUnset -> 19
    BackupStateInconsistent -> 20
    AssertionInvalid -> 21
    AttesterKeyOffCurve -> 22

-- | Why a Malformed line is not an Entry.
lineErrorCode :: LineError -> Int32
lineErrorCode = \case
    LineNotWellFormed _ -> 100
    LineNotJson _ -> 101
    LineNotAnObject -> 102
    LineTrailingContent -> 103
    UnknownMember _ -> 104
    DuplicateMember _ -> 105
    NonStringValue _ -> 106
    MissingMember _ -> 107
    UnknownRevision _ -> 108
    MemberOutsideRevision _ -> 109
    MemberOutsideKind _ -> 110
    UnknownKind _ -> 111
    NotBase64Url _ _ -> 112
    MemberWrongShape _ _ -> 113

-- | Why libcrypto gave no answer. Not a fault in the line.
cryptoErrorCode :: CryptoError -> Int32
cryptoErrorCode = \case
    KeyNotOnCurve -> 200
    ScalarWrongLength _ -> 201
    ScalarRejected -> 202
    SigOutOfRange -> 203
    OpenSslFailed _ -> 204
