{-# LANGUAGE OverloadedStrings, OverloadedRecordDot #-}
-- A real Producer against the Hub, split in two pure halves the shell drives:
--   producer mint KEYHEX PREV_B64URL FIRST COUNT DIR   writes DIR/p-<n> payloads, DIR/lines.jsonl, DIR/hashes
--   producer verify FILE                              folds the file with the upstream verifier; one line per verdict
module Main (main) where

import Data.ByteString (ByteString)
import Data.ByteString qualified as BS
import Data.ByteString.Base64.URL qualified as B64
import Data.Base64.Types (extractBase64)
import Data.ByteString.Char8 qualified as C8
import Data.ByteString.Lazy qualified as LBS
import Data.Char (digitToInt)
import Data.Text qualified as T
import EventChain
import EventChain.Crypto (PrivateKey, hashPayloads, privateKey)
import EventChain.Crypto.Types (LineHash, lineBytesRaw, lineHashFromBytes, lineHashRaw, payloadBytes)
import EventChain.Verify (Verdict (..), verifiedLineHash, verifiedPosition, verify)
import EventChain.Verify.Types (chainPositionIndex)
import System.Environment (getArgs)
import System.Exit (exitFailure)
import System.IO (hPutStrLn, stderr)

main :: IO ()
main = getArgs >>= \case
    ["mint", keyHex, prevB64, first, count, dir] -> mint (C8.pack keyHex) (C8.pack prevB64) (read first) (read count) dir
    ["verify", file] -> verifyFile file
    _ -> hPutStrLn stderr "usage: producer mint KEYHEX PREV_B64URL FIRST COUNT DIR | producer verify FILE" >> exitFailure

mint :: ByteString -> ByteString -> Int -> Int -> FilePath -> IO ()
mint keyHex prevB64 first count dir = do
    key <- either (die . ("key: " <>) . show) pure (privateKey (unhex keyHex))
    prevRaw <- either (die . ("prev: " <>) . T.unpack) pure (B64.decodeBase64UnpaddedUntyped prevB64)
    prev <- either (die . ("prev: " <>) . show) pure (lineHashFromBytes prevRaw)
    let numbers = [first .. first + count - 1]
        payloads = [C8.pack ("payload " <> show n <> " of the end-to-end run\n") | n <- numbers]
        digests = hashPayloads (map payloadBytes payloads)
        events = [ ChainedEvent { entryId = entryId (T.pack ("evt-" <> show n)), payloadHash = h, payloadRef = payloadRef (T.pack ("p-" <> show n)), prevHash = prev, kind = Lifecycle }
                 | (n, h) <- zip numbers digests ]
        produced = chain key prev events
    mapM_ (\(n, p) -> BS.writeFile (dir <> "/p-" <> show n) p) (zip numbers payloads)
    BS.writeFile (dir <> "/lines.jsonl") (BS.concat [lineBytesRaw p.line <> "\n" | p <- produced])
    BS.writeFile (dir <> "/hashes") (C8.unlines [extractBase64 (B64.encodeBase64Unpadded' (lineHashRaw p.lineHash)) | p <- produced])

chain :: PrivateKey -> LineHash -> [ChainedEvent] -> [ProducedLine]
chain _ _ [] = []
chain key prev (event : rest) = case produce key event{prevHash = prev} of
    Right p -> p : chain key p.lineHash rest
    Left err -> error ("produce refused a key libcrypto accepted: " <> show err)

verifyFile :: FilePath -> IO ()
verifyFile file = do
    bytes <- LBS.readFile file
    verdicts <- either (die . ("framing: " <>) . show) pure (verify bytes)
    ok <- mapM report verdicts
    if and ok then pure () else exitFailure
  where
    report (Sound v) = C8.putStrLn (C8.pack (show (chainPositionIndex (verifiedPosition v))) <> " Sound " <> extractBase64 (B64.encodeBase64Unpadded' (lineHashRaw (verifiedLineHash v)))) >> pure True
    report other = putStrLn (show other) >> pure False

unhex :: ByteString -> ByteString
unhex = BS.pack . go . C8.unpack
  where
    go (a : b : r) = fromIntegral (digitToInt a * 16 + digitToInt b) : go r
    go _ = []

die :: String -> IO a
die message = hPutStrLn stderr message >> exitFailure
