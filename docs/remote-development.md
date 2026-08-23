# Mac 开发与 Windows x86 运行端

日常工作流：**Mac 本地仓库改代码**，**GitHub 同步**，公司 Windows 主机做原生 x86 构建、测试和 QEMU。两台机器走 Tailscale SSH。

SSH 代称是 `court-kernel`，不要用其它项目的主机别名。

## 固定环境

| 项目     | 值                                             |
| -------- | ---------------------------------------------- |
| SSH 代称 | `court-kernel`                                 |
| 远程主机 | `<tailscale-or-lan-ip>`（Tailscale）                   |
| SSH 用户 | `<windows-user>`                                |
| 远程目录 | `G:\Court-Kernel`                              |
| 远程系统 | Windows，默认 SSH shell 为 `cmd.exe`           |
| 远程架构 | 原生 AMD64 / Intel64                           |
| Git 远程 | `https://github.com/ICE6332/Court-Kernel.git`  |
| 同步分支 | `main`                                         |
| Mac 仓库 | `<mac-repo>`         |

Unix demo 和 `scripts/run-qemu.sh` 需要 Linux。Windows 上走 WSL2（当前发行版是 Debian，平时可能是 Stopped，用前先启动）。

仓库不保存 SSH 密码。日常只用密钥。

## 首次初始化

Mac `~/.ssh/config`：

```sshconfig
Host court-kernel
  HostName <tailscale-or-lan-ip>
  User <windows-user>
  IdentityFile ~/.ssh/agent/court-kernel_ed25519
  IdentitiesOnly yes
```

连通测试：

```bash
ssh court-kernel "hostname"
ssh court-kernel "echo %PROCESSOR_ARCHITECTURE%"
```

远程目录已经存在，不要重新 `git clone`。每次同步前先看工作区：

```bash
ssh court-kernel "git -C G:\Court-Kernel status -sb"
ssh court-kernel "git -C G:\Court-Kernel remote -v"
```

`git status --short` 只要有输出，就停下来人工处理。不要自动 `git reset --hard`、`git clean` 或 `git stash`。

Windows 上跑 bash 时用 Git for Windows，不要用 PATH 里的 `bash`（那是 WSL）：

```bat
"C:\Program Files\Git\bin\bash.exe" -lc "git -C /g/Court-Kernel status -sb"
```

## 日常同步

Mac 提交并推到 `origin/main` 之后：

```bash
ssh court-kernel "git -C G:\Court-Kernel status --short"
ssh court-kernel "git -C G:\Court-Kernel pull --ff-only origin main"
ssh court-kernel "git -C G:\Court-Kernel rev-parse --short HEAD"
```

Windows 上改的代码同样先提交推送，再在 Mac 上 `git pull --ff-only origin main`。只允许快进，两边不要分叉 `main`。

远程的 `target/`、`build/`、`third_party/limine/` 是机器本地产物，已在 `.gitignore` 里，不要从 Mac 覆盖。

## 在 Windows 上构建

便携测试（Windows 原生即可）：

```bat
cd /d G:\Court-Kernel
cargo test --workspace
```

Unix demo / QEMU 需要 WSL2：

```bat
wsl -d Debian --cd /mnt/g/Court-Kernel -e bash -lc "cargo test --workspace"
wsl -d Debian --cd /mnt/g/Court-Kernel -e bash -lc "cargo test -p court-hosted-linux --test mvp0b_demo"
wsl -d Debian --cd /mnt/g/Court-Kernel -e bash -lc "bash scripts/run-qemu.sh"
```

QEMU 脚本在 `/dev/kvm` 可写时会开 KVM；没有 KVM 时走 TCG。

## 故障排查

```bash
tailscale ping <tailscale-or-lan-ip>
ssh court-kernel "hostname"
ssh court-kernel "git -C G:\Court-Kernel status -sb"
ssh court-kernel "git -C G:\Court-Kernel rev-parse HEAD"
```
