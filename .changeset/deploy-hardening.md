---
"frontal-sandbox": patch
---

Harden `infrastructure/scripts/restore.sh`: refuse backup archives that contain symlinks, absolute paths, or parent references before extracting anything, extract into a scratch directory and swap it into place only after the sandbox/ tree verifies, and move the live state aside instead of deleting it so a failed restore puts the previous state back. Add `umask 077` to `backup.sh` so state archives (which contain keys and session data) are created owner-only.
