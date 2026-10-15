{- | The C ABI the Hub calls, declared in @include/eventchain_acceptor.h@.
Every function here is a thin edge: bytes are copied in, a library function
or "EventChain.Acceptor" is called, and its result is copied out with a
status code. No protocol logic lives here.

The Hub key crosses once, at load, and comes back as an opaque handle: a
'StablePtr' to the parsed 'PrivateKey'. The scalar itself never crosses
back.
-}
module EventChain.Acceptor.FFI () where

import Data.ByteString (ByteString)
import Data.ByteString qualified as BS
import Data.ByteString.Unsafe qualified as BSU
import Data.Int (Int32)
import Data.Word (Word64, Word8)
import EventChain.Acceptor (AcceptError (..), Coded (..), accept, code)
import EventChain.Crypto (PrivateKey, privateKey, privateKeyPublic, sign)
import EventChain.Crypto.Types (claimedKeyRaw, lineHashFromBytes, sha256Length, sigRaw, signedBytes)
import Foreign.C.Types (CInt (..), CSize (..))
import Foreign.Marshal.Utils (copyBytes)
import Foreign.Ptr (Ptr, castPtr)
import Foreign.StablePtr (StablePtr, deRefStablePtr, freeStablePtr, newStablePtr)
import Foreign.Storable (poke)

foreign export ccall ec_key_load :: Ptr Word8 -> CSize -> Ptr (StablePtr PrivateKey) -> IO CInt
foreign export ccall ec_key_free :: StablePtr PrivateKey -> IO ()
foreign export ccall ec_key_public :: StablePtr PrivateKey -> Ptr Word8 -> IO ()
foreign export ccall ec_sign :: StablePtr PrivateKey -> Ptr Word8 -> CSize -> Ptr Word8 -> IO CInt
foreign export ccall ec_accept :: Word64 -> Ptr Word8 -> Ptr Word8 -> CSize -> Ptr Int32 -> Ptr Int32 -> Ptr Word8 -> IO CInt

-- The status codes, as the header numbers them.
ok, keyRejected, cryptoFailed, badArgument, notOneLine :: CInt
ok = 0
keyRejected = 1
cryptoFailed = 2
badArgument = 3
notOneLine = 4

-- | Copy a caller-owned buffer into a ByteString the library can hold.
readBytes :: Ptr Word8 -> CSize -> IO ByteString
readBytes p n = BS.packCStringLen (castPtr p, fromIntegral n)

-- | Copy a library result into a caller-owned buffer of exactly its length.
writeBytes :: Ptr Word8 -> ByteString -> IO ()
writeBytes out bs = BSU.unsafeUseAsCStringLen bs $ \(src, n) -> copyBytes out (castPtr src) n

ec_key_load :: Ptr Word8 -> CSize -> Ptr (StablePtr PrivateKey) -> IO CInt
ec_key_load keyPtr keyLen out = do
    scalar <- readBytes keyPtr keyLen
    case privateKey scalar of
        Left _ -> pure keyRejected
        Right key -> do
            poke out =<< newStablePtr key
            pure ok

ec_key_free :: StablePtr PrivateKey -> IO ()
ec_key_free = freeStablePtr

ec_key_public :: StablePtr PrivateKey -> Ptr Word8 -> IO ()
ec_key_public handle out = do
    key <- deRefStablePtr handle
    writeBytes out (claimedKeyRaw (privateKeyPublic key))

ec_sign :: StablePtr PrivateKey -> Ptr Word8 -> CSize -> Ptr Word8 -> IO CInt
ec_sign handle msgPtr msgLen out = do
    key <- deRefStablePtr handle
    msg <- readBytes msgPtr msgLen
    case sign key (signedBytes msg) of
        Left _ -> pure cryptoFailed
        Right sig -> do
            writeBytes out (sigRaw sig)
            pure ok

ec_accept :: Word64 -> Ptr Word8 -> Ptr Word8 -> CSize -> Ptr Int32 -> Ptr Int32 -> Ptr Word8 -> IO CInt
ec_accept count headPtr linePtr lineLen outVerdict outFault outHash = do
    headBytes <- readBytes headPtr (fromIntegral sha256Length)
    line <- readBytes linePtr lineLen
    case lineHashFromBytes headBytes of
        Left _ -> pure badArgument
        Right headHash -> case accept count headHash line of
            Left HeadNotGenesis -> pure badArgument
            Left (NotOneLine _) -> pure notOneLine
            Right judgement -> do
                let Coded verdict fault hash = code judgement
                poke outVerdict verdict
                poke outFault fault
                writeBytes outHash hash
                pure ok
