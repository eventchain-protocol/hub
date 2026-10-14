{- | The C ABI the Hub calls. Every function here is a thin edge over an
eventchain library function: bytes are copied in, the library is called, and
its result is copied out with a status code. No protocol logic lives here.
-}
module EventChain.Acceptor.FFI () where

import Data.ByteString (ByteString)
import Data.ByteString qualified as BS
import Data.ByteString.Unsafe qualified as BSU
import EventChain.Crypto (PrivateKey, privateKey, privateKeyPublic, sign)
import EventChain.Crypto.Types (claimedKeyRaw, sigRaw, signedBytes)
import Foreign.C.Types (CInt (..), CSize (..))
import Foreign.Marshal.Utils (copyBytes)
import Foreign.Ptr (Ptr, castPtr)
import Data.Word (Word8)

foreign export ccall ec_public_key :: Ptr Word8 -> CSize -> Ptr Word8 -> IO CInt
foreign export ccall ec_sign :: Ptr Word8 -> CSize -> Ptr Word8 -> CSize -> Ptr Word8 -> IO CInt

ok, keyRejected, cryptoFailed :: CInt
ok = 0
keyRejected = 1
cryptoFailed = 2

-- | Copy a caller-owned buffer into a ByteString the library can hold.
readBytes :: Ptr Word8 -> CSize -> IO ByteString
readBytes p n = BS.packCStringLen (castPtr p, fromIntegral n)

-- | Copy a library result into a caller-owned buffer of exactly its length.
writeBytes :: Ptr Word8 -> ByteString -> IO ()
writeBytes out bs = BSU.unsafeUseAsCStringLen bs $ \(src, n) -> copyBytes out (castPtr src) n

withKey :: Ptr Word8 -> CSize -> (PrivateKey -> IO CInt) -> IO CInt
withKey keyPtr keyLen k = do
    scalar <- readBytes keyPtr keyLen
    case privateKey scalar of
        Left _ -> pure keyRejected
        Right key -> k key

ec_public_key :: Ptr Word8 -> CSize -> Ptr Word8 -> IO CInt
ec_public_key keyPtr keyLen out = withKey keyPtr keyLen $ \key -> do
    writeBytes out (claimedKeyRaw (privateKeyPublic key))
    pure ok

ec_sign :: Ptr Word8 -> CSize -> Ptr Word8 -> CSize -> Ptr Word8 -> IO CInt
ec_sign keyPtr keyLen msgPtr msgLen out = withKey keyPtr keyLen $ \key -> do
    msg <- readBytes msgPtr msgLen
    case sign key (signedBytes msg) of
        Left _ -> pure cryptoFailed
        Right sig -> do
            writeBytes out (sigRaw sig)
            pure ok
