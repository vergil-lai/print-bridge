# Remote task examples

这里放的是 PrintBridge 远程任务轮询的服务端示例。它们模拟业务服务器，给 PrintBridge 提供同一个远程任务接口：

```text
GET  /print-task  拉取任务
POST /print-task  接收状态上报
```

默认监听地址：

```text
http://127.0.0.1:18080/print-task
```

默认 Bearer Token：

```text
dev-token
```

在 PrintBridge 设置页的远程任务配置里填写：

```text
任务接口 URL: http://127.0.0.1:18080/print-task
Bearer Token: dev-token
```

## Node

运行：

```bash
node examples/remote-task/node/remote-task-server.mjs
```

默认返回一个 PDF 单任务，打印文件来自 JSSDK examples assets：

```text
https://raw.githubusercontent.com/vergil-lai/print-bridge-jssdk/main/examples/assets/printbridge-a4-sample.pdf
```

需要修改端口、Token 或文件 URL 时，直接改示例代码里的常量。

## PHP

运行：

```bash
php -S 127.0.0.1:18080 examples/remote-task/php/remote-task-server.php
```

默认返回一个图片单任务，打印文件来自 JSSDK examples assets：

```text
https://raw.githubusercontent.com/vergil-lai/print-bridge-jssdk/main/examples/assets/printbridge-a4-sample.jpg
```

需要修改 Token、文件 URL 或格式时，直接改示例代码里的变量。

## Go

运行：

```bash
go run examples/remote-task/go/remote-task-server.go
```

默认返回一个批量任务，包含 JSSDK examples assets 里的 PDF 和 JPG：

```text
https://raw.githubusercontent.com/vergil-lai/print-bridge-jssdk/main/examples/assets/printbridge-a4-sample.pdf
https://raw.githubusercontent.com/vergil-lai/print-bridge-jssdk/main/examples/assets/printbridge-a4-sample.jpg
```

需要修改端口、Token 或文件 URL 时，直接改示例代码里的常量。

## 连接测试

PrintBridge 保存远程配置或点击测试连接时，会发送带有下面请求头的测试请求：

```text
X-PrintBridge-Test: true
```

三个示例都会对测试 `GET` 和测试 `POST` 返回 `204 No Content`，不会发放真实任务，也不会写状态。

## 状态上报

PrintBridge 拉取任务后，会把远程状态通过 `POST /print-task` 上报回来。示例会把收到的 JSON 打印到终端。

当前会收到的状态主要是：

```text
accepted
success
failed
```

`success` 表示任务已提交到系统打印队列，不表示打印机已经真实出纸。

## 重复拉取

这些示例不会保存状态；每次 `GET` 都会返回同一个固定任务。PrintBridge 会按 `job_id` 记录远程任务，已经处理过的 `job_id` 不会重复进入打印队列。
