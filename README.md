# ctxsync

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
[![PyPI](https://badge.fury.io/py/ctxsync.svg)](https://pypi.org/project/ctxsync/)
[![Release](https://img.shields.io/github/release/jahwag/ctxsync.svg)](https://github.com/jahwag/ctxsync/releases)
[![Build Status](https://github.com/jahwag/ctxsync/actions/workflows/python-package.yml/badge.svg)](https://github.com/jahwag/ctxsync/actions/workflows/python-package.yml)
[![Issues](https://img.shields.io/github/issues/jahwag/ctxsync)](https://github.com/jahwag/ctxsync/issues)
[![Code Style: Black](https://img.shields.io/badge/code%20style-black-000000.svg)](https://github.com/psf/black)
[![Dependencies](https://img.shields.io/librariesio/github/jahwag/ctxsync)](https://github.com/jahwag/ctxsync/network/dependencies)
[![Last Commit](https://img.shields.io/github/last-commit/jahwag/ctxsync.svg)](https://github.com/jahwag/ctxsync/commits/main)
[![Sponsor jahwag](https://img.shields.io/badge/Sponsor-♥-red)](https://github.com/sponsors/jahwag)


ctxsync (formerly known as ClaudeSync) bridges your local development environment with Claude.ai projects, enabling seamless synchronization to enhance your AI-powered workflow.

> **Rust port available:** a feature-equivalent Rust implementation lives in the [`rust/`](rust/) directory. It shares the same configuration files and encrypted session-key storage as the Python version, so the two are interchangeable. See [rust/README.md](rust/README.md) for build instructions.

> **macOS menu bar app:** a native SwiftUI companion app lives in [`swift/`](swift/) — login, project setup, manual and FSEvents-driven auto-sync from the menu bar, sharing the same config files and Keychain entry as the Rust CLI. See [swift/README.md](swift/README.md).

![ctxsync in Action](claudesync.gif)

## ⚠️ Disclaimer

ctxsync is an independent, open-source project **not affiliated** with Anthropic or Claude.ai. By using ctxsync, you agree to:

1. Use it at your own risk.
2. Acknowledge potential violation of Anthropic's Terms of Service.
3. Assume responsibility for any consequences.
4. Understand that Anthropic does not support this tool.

Please review [Anthropic's Terms of Service](https://www.anthropic.com/legal/consumer-terms) before using ctxsync.

## 🌟 Features

- **File sync**: Synchronize local files with [Claude.ai projects](https://www.anthropic.com/news/projects).
- **Cross-Platform**: Compatible with [Windows, macOS, and Linux](https://github.com/jahwag/ctxsync/releases).
- **Configurable**: Plenty of [configuration options](https://github.com/jahwag/ctxsync/wiki/Quick-reference).
- **Integrate**: Designed to be easy to integrate into your pipelines.
- **Secure**: Ensures data privacy and security.

## ⚙️ Prerequisites

### 📄 Supported Claude.ai plans

| [Plan](https://www.anthropic.com/pricing)   | Supported |
|--------|-----------|
| Pro    | ✅        |
| Team   | ✅        |
| Free   | ❌        |

### 🔑 SSH Key

Ensure you have an SSH key for secure credential storage. Follow [GitHub's guide](https://docs.github.com/en/authentication/connecting-to-github-with-ssh) to generate and add your SSH key.

### 💻 Software

- **Python**: ≥ [3.10](https://www.python.org/downloads/)
- **pip**: [Python package installer](https://pip.pypa.io/en/stable/installation/)

## 🚀 Quick Start

1. **Install ctxsync**
    ```shell
    pip install ctxsync
    ```

2. **Authenticate**
    ```shell
    ctxsync auth login
    ```

3. **Create a Project**
    ```shell
    ctxsync project create
    ```

4. **Start Syncing***
    ```shell
    ctxsync push
    ```
    **This is a one-way sync. Files not present locally will be removed from the Claude.ai project unless pruning is [disabled](https://github.com/jahwag/ctxsync/wiki/Quick-reference#pruning-remote).*

📚 [Detailed Guides & FAQs](https://github.com/jahwag/ctxsync/wiki)

## 🤝 Support & Contribute

Enjoying ctxsync? Support us by:

- ⭐ [Starring the Repository](https://github.com/jahwag/ctxsync)
- 🐛 [Reporting Issues](https://github.com/jahwag/ctxsync/issues)
- 🌍 [Contributing](CONTRIBUTING.md)
- 💬 [Join Our Discord](https://discord.gg/pR4qeMH4u4)
- 💖 [Sponsor Us](https://github.com/sponsors/jahwag)

Your contributions help improve ctxsync!

---

[Contributors](https://github.com/jahwag/ctxsync/graphs/contributors) • [License](https://github.com/jahwag/ctxsync/blob/master/LICENSE) • [Report Bug](https://github.com/jahwag/ctxsync/issues) • [Request Feature](https://github.com/jahwag/ctxsync/issues/new?labels=enhancement&template=feature_request.md)• [Sponsor](https://github.com/sponsors/jahwag)
