# Reference SigV4 for the exact request FiveMClip will send, computed by
# botocore. Whatever this prints, the Rust signer must reproduce.
from botocore.auth import SigV4Auth
from botocore.awsrequest import AWSRequest
from botocore.credentials import Credentials
import datetime

creds = Credentials("AKIAIOSFODNN7EXAMPLE", "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY")
url = "https://abc123.r2.cloudflarestorage.com/clips/FiveMClip/Clip_2026-09-12_13-13-32.mp4"
req = AWSRequest(method="PUT", url=url, headers={
    "host": "abc123.r2.cloudflarestorage.com",
    "content-type": "video/mp4",
    "x-amz-content-sha256": "UNSIGNED-PAYLOAD",
    "x-amz-date": "20260912T131332Z",
})
auth = SigV4Auth(creds, "s3", "auto")
# Pin the clock so the result is reproducible.
req.context["timestamp"] = "20260912T131332Z"
cr = auth.canonical_request(req)
sts = auth.string_to_sign(req, cr)
sig = auth.signature(sts, req)
print("=== canonical request ===");  print(cr)
print("=== string to sign ===");     print(sts)
print("=== signature ===");          print(sig)

# And a key with characters that must be percent-encoded.
req2 = AWSRequest(method="PUT", url="https://abc123.r2.cloudflarestorage.com/clips/a%20b%2Bc/Clip%20%231.mp4", headers={
    "host": "abc123.r2.cloudflarestorage.com",
    "content-type": "video/mp4",
    "x-amz-content-sha256": "UNSIGNED-PAYLOAD",
    "x-amz-date": "20260912T131332Z",
})
req2.context["timestamp"] = "20260912T131332Z"
cr2 = auth.canonical_request(req2)
print("=== encoded-key canonical path ==="); print(cr2.split("\n")[1])
print("=== encoded-key signature ===");      print(auth.signature(auth.string_to_sign(req2, cr2), req2))
