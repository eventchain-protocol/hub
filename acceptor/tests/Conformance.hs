{- | The acceptor's conformance test: every committed eventchain vector pushed
through the exported C function one line at a time, asserting the verdict
the Verifier gives the same line when it folds the whole file.

The call goes through @ec_accept@ itself, imported back from the foreign
export compiled into this executable, so the marshalling is under test and
not only the Haskell behind it. The expected side is 'verify' over the file:
bytes in, codes out, no mock and no hand-written table of expectations.

Between lines the head advances by hashing the previous line's raw bytes,
which is what the fold does whatever its verdict was, so an Unsound line in
a vector is followed exactly as the Verifier follows it. A Malformed verdict
is always the fold's last, so the comparison stops there.
-}
module Main (main) where

import Control.Monad (filterM, forM, forM_, unless, when)
import Data.ByteString (ByteString)
import Data.ByteString qualified as BS
import Data.ByteString.Lazy qualified as LBS
import Data.ByteString.Unsafe qualified as BSU
import Data.Int (Int32)
import Data.List (isPrefixOf, sort)
import Data.Word (Word64, Word8)
import EventChain.Acceptor (Coded (..), Judgement (..), code)
import EventChain.Acceptor.FFI ()
import EventChain.Crypto (hashLines)
import EventChain.Crypto.Types (lineBytes, lineHashRaw, sha256Length)
import EventChain.Verify (Verdict (..), verify)
import Foreign.C.Types (CInt (..), CSize (..))
import Foreign.Marshal.Alloc (alloca, allocaBytes)
import Foreign.Ptr (Ptr, castPtr)
import Foreign.Storable (peek)
import System.Directory (doesDirectoryExist, listDirectory)
import System.Exit (exitFailure)
import System.FilePath (takeExtension, (</>))
import System.IO (hPutStrLn, stderr)

foreign import ccall "ec_accept"
    c_accept :: Word64 -> Ptr Word8 -> Ptr Word8 -> CSize -> Ptr Int32 -> Ptr Int32 -> Ptr Word8 -> IO CInt

main :: IO ()
main = do
    dir <- vectorsDir
    files <- jsonlFiles dir
    when (null files) (failWith ("no .jsonl vectors under " <> dir))
    forM_ files $ \path -> do
        file <- BS.readFile path
        expected <- either (\e -> failWith (path <> ": the Verifier could not frame it: " <> show e)) pure (verify (LBS.fromStrict file))
        let lns = splitLines file
        when (length expected > length lns) (failWith (path <> ": more verdicts than lines"))
        checkFile path (zip3 [0 ..] lns expected)
    lifecycle <- BS.readFile (dir </> "v0-lifecycle.jsonl")
    headMatters (splitLines lifecycle)
    genesisOnly (splitLines lifecycle)
    putStrLn (show (length files) <> " vectors through the C ABI; every verdict the Verifier's")

-- | Each line against its whole-file verdict, with the head advanced by raw bytes.
checkFile :: FilePath -> [(Word64, ByteString, Verdict)] -> IO ()
checkFile path = go genesis
  where
    go _ [] = pure ()
    go headBytes ((count, line, verdict) : rest) = do
        (status, got) <- callAccept count headBytes line
        unless (status == 0) (failWith (path <> " line " <> show count <> ": status " <> show status))
        let want = code (Judged verdict)
        unless (got == want) (failWith (path <> " line " <> show count <> ": the ABI said " <> show got <> ", the Verifier said " <> show want))
        unless (null rest) (go (rawHash path count line) rest)

-- | The second lifecycle line judged against the genesis head is a chain mismatch: the head argument is used.
headMatters :: [ByteString] -> IO ()
headMatters lns = case lns of
    (_ : second : _) -> do
        (status, got) <- callAccept 1 genesis second
        unless (status == 0 && got == Coded 2 1 noHash) (failWith ("a line judged against the wrong head should be Unsound with a chain mismatch; got " <> show (status, got)))
    _ -> failWith "v0-lifecycle.jsonl has fewer than two lines"

-- | A count of zero with any head but the sentinel is refused as a bad argument.
genesisOnly :: [ByteString] -> IO ()
genesisOnly lns = case lns of
    (first : _) -> do
        (status, _) <- callAccept 0 (rawHash "v0-lifecycle.jsonl" 0 first) first
        unless (status == 3) (failWith ("a count of zero with a non-genesis head should be EC_BAD_ARGUMENT; got status " <> show status))
    _ -> failWith "v0-lifecycle.jsonl is empty"

callAccept :: Word64 -> ByteString -> ByteString -> IO (CInt, Coded)
callAccept count headBytes line =
    BSU.unsafeUseAsCStringLen headBytes $ \(headPtr, _) ->
        BSU.unsafeUseAsCStringLen line $ \(linePtr, lineLen) ->
            alloca $ \verdictPtr -> alloca $ \faultPtr -> allocaBytes sha256Length $ \hashPtr -> do
                status <- c_accept count (castPtr headPtr) (castPtr linePtr) (fromIntegral lineLen) verdictPtr faultPtr hashPtr
                verdict <- peek verdictPtr
                fault <- peek faultPtr
                hash <- BS.packCStringLen (castPtr hashPtr, sha256Length)
                pure (status, Coded verdict fault hash)

-- | SHA-256 of a line's raw bytes, as the fold advances its head.
rawHash :: FilePath -> Word64 -> ByteString -> ByteString
rawHash path count line = case lineBytes line of
    Left err -> error (path <> " line " <> show count <> ": not line content: " <> show err)
    Right ln -> case hashLines [ln] of
        [h] -> lineHashRaw h
        hs -> error (path <> ": " <> show (length hs) <> " digests for one line")

genesis, noHash :: ByteString
genesis = BS.replicate sha256Length 0
noHash = genesis

-- | The file's lines without terminators. A trailing terminator ends the last line rather than opening an empty one.
splitLines :: ByteString -> [ByteString]
splitLines file = case BS.split 0x0a file of
    pieces | not (null pieces) && BS.null (last pieces) -> init pieces
    pieces -> pieces

-- | The vectors inside the eventchain checkout cabal made for the pinned commit.
vectorsDir :: IO FilePath
vectorsDir = do
    let root = "dist-newstyle" </> "src"
    entries <- listDirectory root
    found <- filterM doesDirectoryExist [root </> e </> "vectors" | e <- entries, "eventchain-" `isPrefixOf` e]
    case found of
        [one] -> pure one
        [] -> failWith ("no eventchain checkout with a vectors/ directory under " <> root)
        many -> failWith ("more than one eventchain checkout under " <> root <> ": " <> show many)

-- | Every .jsonl file below the directory, in a fixed order.
jsonlFiles :: FilePath -> IO [FilePath]
jsonlFiles dir = do
    entries <- sort <$> listDirectory dir
    nested <- forM entries $ \e -> do
        let path = dir </> e
        isDir <- doesDirectoryExist path
        if isDir
            then jsonlFiles path
            else pure [path | takeExtension path == ".jsonl"]
    pure (concat nested)

failWith :: String -> IO a
failWith msg = hPutStrLn stderr msg >> exitFailure
