# 部署

## 桌面版

Windows、macOS 和 Linux GUI 安装包提供 `print-bridge-desktop` 软件包及 `print-bridge` 可执行文件。无参数启动 GUI。桌面产品不支持 `serve`。

## Linux headless

headless deb/rpm 的软件包名为 `print-bridge-server`，安装后自动：

1. 创建无 home 的 `printbridge` 系统用户和组；
2. 准备 `/etc/print-bridge`、`/var/lib/print-bridge` 和 `/run/print-bridge`；
3. 安装并启用 `print-bridge.service`；
4. 以 `printbridge` 用户执行 `/usr/bin/print-bridge serve`。

```bash
sudo apt install ./print-bridge-server_VERSION_ARCH.deb
systemctl status print-bridge
journalctl -u print-bridge -f
```

手工诊断可以执行：

```bash
sudo -u printbridge /usr/bin/print-bridge serve
print-bridge status
print-bridge doctor
```

`doctor` 执行只读环境检查：配置有效性、数据目录权限、Agent IPC 可达性、端口可用性、打印机可见性、浏览器与 Office 转换器是否存在，以及远程连接可达性（headless 产品额外检查 systemd 服务状态）。

没有 `print-bridge serve install` 或 `uninstall`。服务生命周期由包管理器维护；升级保留配置和状态，只有 purge 才删除数据。

## 互斥性

GUI 与 headless 都占用 `/usr/bin/print-bridge`，因此软件包双向声明 `Conflicts` 和 `Provides: print-bridge`。安装另一产品会明确失败，不会自动卸载或替换已安装产品。

## 健康检查与诊断

产品不提供 `/health` 或其他 REST API。使用以下方式检查：

- `systemctl status print-bridge`
- `print-bridge status`（本地 IPC）
- `print-bridge doctor`（环境检查，见上文）
- WebSocket `/ws` ping/pong
- systemd `Type=notify` 的 READY/STOPPING 状态

## 依赖

headless 需要 CUPS client、LibreOffice，以及可由系统用户执行的 Chrome/Chromium。打印机必须对 `printbridge` 用户可见；浏览器和 LibreOffice 的运行时文件必须写入 `/var/lib/print-bridge` 或临时目录，不能依赖个人 home。
