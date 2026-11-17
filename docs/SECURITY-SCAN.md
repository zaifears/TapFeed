# Security scanning

Every TapFeed release is scanned by [VirusTotal](https://www.virustotal.com/) with about 70 antivirus engines. The result is recorded here by the [VirusTotal scan workflow](../.github/workflows/virustotal.yml), and the newest one drives the badge in the README.

Compare the SHA-256 below with your download (`certutil -hashfile TapFeed.exe SHA256` in a Windows terminal) to confirm you have the exact file that was scanned.

TapFeed is not code-signed yet, and it bundles a small Python runtime for the camera helper. A few antivirus engines flag programs like that out of caution, so a low count is usually a false positive; the full report names each engine.

| Release | File | Detections | Report | SHA-256 |
|---|---|---|---|---|
| [`v2.0.0`](https://github.com/zaifears/TapFeed/releases/tag/v2.0.0) | `TapFeed.exe` | 1 / 70 | [report](https://www.virustotal.com/gui/file/c1836268c1bf60fad9b860998b7fea9fc112e3d3dbb9273ed7d04534ce574ef3) | `c1836268c1bf60fad9b860998b7fea9fc112e3d3dbb9273ed7d04534ce574ef3` |
