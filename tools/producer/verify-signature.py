# Verify one heartbeat statement (argv[2], a file) under the Hub key (argv[1], base64url SEC1 compressed) with openssl.
import base64, json, subprocess, sys, tempfile, os

def b64u(s):
    return base64.urlsafe_b64decode(s + "=" * (-len(s) % 4))

def der_int(b):
    b = b.lstrip(b"\x00") or b"\x00"
    if b[0] & 0x80:
        b = b"\x00" + b
    return b"\x02" + bytes([len(b)]) + b

key, path = sys.argv[1], sys.argv[2]
members = json.loads(open(path, "rb").read())
sig = b64u(members.pop("signature"))
message = json.dumps(dict(sorted(members.items())), separators=(",", ":"), ensure_ascii=False).encode()
seq = der_int(sig[:32]) + der_int(sig[32:])
der_sig = b"\x30" + bytes([len(seq)]) + seq
spki = bytes.fromhex("3039301306072a8648ce3d020106082a8648ce3d030107032200") + b64u(key)
d = tempfile.mkdtemp()
open(os.path.join(d, "pub.der"), "wb").write(spki)
open(os.path.join(d, "sig.der"), "wb").write(der_sig)
open(os.path.join(d, "msg"), "wb").write(message)
r = subprocess.run(["openssl", "dgst", "-sha256", "-verify", os.path.join(d, "pub.der"), "-keyform", "DER", "-signature", os.path.join(d, "sig.der"), os.path.join(d, "msg")], capture_output=True, text=True)
print((r.stdout + r.stderr).strip())
