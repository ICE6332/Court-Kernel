# Mac 开发与 Windows x86 运行端

日常工作流：**Mac 本地仓库改代码**，**GitHub 同步**，一台原生 x86 Windows 主机做构建、测试和 QEMU。两台机器走私有组网 SSH。

本文件只描述流程。主机地址、账号和密钥留在本机 `~/.ssh/config`，不要写进仓库。

## 约定

| 项目     | 值                                            |
| -------- | --------------------------------------------- |
| SSH 代称 | `court-kernel`（只出现在本机 SSH config）     |
| 远程系统 | Windows，默认 SSH shell 为 `cmd.exe`          |
| 远程架构 | 原生 AMD64                                    |
| Git 远程 | `https://github.com/ICE6332/Court-Kernel.git` |
| 同步分支 | `main`                                        |

Unix demo 和 `scripts/run-qemu.sh` 需要 Linux。Windows 上走 WSL2。仓库不保存 SSH 密码或私钥。

## 本机 SSH config（示例）

把真实 `HostName` / `User` / `IdentityFile` 填在本机，不要提交：

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

远程目录按你的实际工作树填写。下面用 `G:\Court-Kernel` 仅作示例。

## 日常同步

每次同步前先看远程工作区。`git status --short` 有输出就停，不要自动 `git reset --hard`、`git clean` 或 `git stash`。

```bash
ssh court-kernel "git -C G:\Court-Kernel status --short"
ssh court-kernel "git -C G:\Court-Kernel pull --ff-only origin main"
ssh court-kernel "git -C G:\Court-Kernel rev-parse --short HEAD"
```

只允许快进。Windows 上需要 bash 时用 Git for Windows：

```bat
"C:\Program Files\Git\bin\bash.exe" -lc "git -C /g/Court-Kernel status -sb"
```

## 在 Windows 上构建

```bat
cd /d G:\Court-Kernel
cargo test --workspace
```

Unix demo / QEMU 走 WSL2：

```bat
wsl -d Debian --cd /mnt/g/Court-Kernel -e bash -lc "cargo test --workspace"
wsl -d Debian --cd /mnt/g/Court-Kernel -e bash -lc "cargo test -p court-hosted-linux --test mvp0b_demo"
wsl -d Debian --cd /mnt/g/Court-Kernel -e bash -lc "bash scripts/run-qemu.sh"
```
