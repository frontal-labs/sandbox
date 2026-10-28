# Frontal Sandbox Managed Images

Dockerfiles for Sandbox Managed Images published to Frontal Container Registry under `frontal/sandbox/*`.

| Image | Base | Contents |
|-------|------|----------|
| `frontal/sandbox/universal:latest` | `frontal/sandbox/ubuntu` | Node.js LTS (24), Python (3.14), coding agents, utilities |
| `frontal/sandbox/node:22\|24\|26` | `frontal/sandbox/ubuntu` | Node.js (major pinned), pnpm |
| `frontal/sandbox/python:3.14` | `frontal/sandbox/ubuntu` | Python 3.14 (pinned), pip, venv, uv |
| `frontal/sandbox/arch:latest` | `archlinux:latest` | Arch Linux, yay (AUR), base-devel, git |
| `frontal/sandbox/ubuntu:latest` | `ubuntu:26.04` | Ubuntu + sudo |
| `frontal/sandbox/node:al-22\|24\|26` | `al-base` | Node.js (major pinned), pnpm |
| `frontal/sandbox/python:al-3.13.1` | `al-base` | Python 3.13.1, pip, venv, uv |

All images are built for `linux/amd64`.

## Building locally

```bash
cd images

# Build all images
docker buildx bake

# Build a single image
docker buildx bake node-24
docker buildx bake universal
```
