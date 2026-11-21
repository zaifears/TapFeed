# Security scanning

Every TapFeed release is scanned by [VirusTotal](https://www.virustotal.com/) with about 70 antivirus engines. The result is recorded here by the [VirusTotal scan workflow](../.github/workflows/virustotal.yml), and the newest one drives the badge in the README.

Compare the SHA-256 below with your download (`certutil -hashfile TapFeed.exe SHA256` in a Windows terminal) to confirm you have the exact file that was scanned.

TapFeed is not code-signed yet, and it bundles a small Python runtime for the camera helper. A few antivirus engines flag programs like that out of caution, so a low count is usually a false positive; the full report names each engine.

| Release | File | Detections | Report | SHA-256 |
|---|---|---|---|---|
| [`v2.2.0`](https://github.com/zaifears/TapFeed/releases/tag/v2.2.0) | `TapFeed.exe` | 1 / 70 | [report](https://www.virustotal.com/gui/file/eb4e4265482f385a57c3e3b1a5e5295154c2275e26baf12339c58b41602c36c5) | `eb4e4265482f385a57c3e3b1a5e5295154c2275e26baf12339c58b41602c36c5` |
| [`v2.1.0`](https://github.com/zaifears/TapFeed/releases/tag/v2.1.0) | `TapFeed.exe` | 1 / 67 | [report](https://www.virustotal.com/gui/file/1e1cd1fa08d75a847fe6be76135a75718e3f84f554816f46e43dc91a2b33dc0f) | `1e1cd1fa08d75a847fe6be76135a75718e3f84f554816f46e43dc91a2b33dc0f` |
| [`v2.0.0`](https://github.com/zaifears/TapFeed/releases/tag/v2.0.0) | `TapFeed.exe` | 1 / 70 | [report](https://www.virustotal.com/gui/file/c1836268c1bf60fad9b860998b7fea9fc112e3d3dbb9273ed7d04534ce574ef3) | `c1836268c1bf60fad9b860998b7fea9fc112e3d3dbb9273ed7d04534ce574ef3` |
