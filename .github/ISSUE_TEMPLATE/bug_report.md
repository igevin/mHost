---
name: Bug Report
about: 创建 bug 报告以帮助我们改进
title: "[Bug] "
labels: bug
assignees: ''
---

## 描述

清晰简洁地描述 bug 是什么。

## 复现步骤

一步步说明触发 bug 的步骤：

1. 打开 Settings → ...
2. 点击 '...'
3. 滚动到 '...'
4. 看到错误

## 期望行为

清晰简洁地描述你期望发生什么。

## 实际行为

清晰简洁地描述实际发生了什么，包括错误信息、截图等。

## 环境

- **mHost 版本**：在 Settings → About 查看（例如 v0.1.0）
- **操作系统**：macOS 14.5 / Windows 11 / Ubuntu 22.04
- **架构**：Apple Silicon (M1/M2/M3) / Intel x86_64
- **运行模式**：Hosts 模式 / DNS 模式 / 两者都有

## DNS 模式补充信息（如适用）

- DNS Mode 状态：Running / Stopped
- 监听端口：53（macOS 经 mhost-dns-proxy）/ 1053
- 上游 DNS：8.8.8.8 / 系统默认
- 启用的 DNS Profile 数量
- 规则总数
- `networksetup -getdnsservers` 输出（macOS）

## 日志

如有控制台 / 终端输出，请贴关键行：

```
[粘贴日志]
```

## 截图 / 录屏

如适用，添加截图帮助解释问题。

## 额外信息

任何其他有助于理解问题的上下文。
