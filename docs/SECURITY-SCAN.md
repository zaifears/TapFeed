# Security scanning

Every TapFeed release is scanned by [VirusTotal](https://www.virustotal.com/) with about 70 antivirus engines. The result is recorded here by the [VirusTotal scan workflow](../.github/workflows/virustotal.yml), and the newest one drives the badge in the README.

Compare the SHA-256 below with your download (`certutil -hashfile TapFeed.exe SHA256` in a Windows terminal) to confirm you have the exact file that was scanned.

TapFeed is not code-signed yet, and it bundles a small Python runtime for the camera helper. A few antivirus engines flag programs like that out of caution, so a low count is usually a false positive; the full report names each engine.

| Release | File | Detections | Report | SHA-256 |
|---|---|---|---|---|
