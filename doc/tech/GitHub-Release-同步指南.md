# GitHub 私有项目 Release 同步到公开项目完整指南

## 背景与需求

在实际开发中，很多团队采用这样的架构：

- **私有仓库**：存放源代码，所有 Release（包含编译后的安装包、二进制文件等）都在私有仓库发布
- **公开仓库**：作为私有仓库的 GitHub Pages 项目，对外展示

核心需求是：**在私有项目中发布 Release 后，自动将 Release 内容同步到公开项目中，让用户可以从公开项目下载编译产物，同时不泄露源代码。**

---

## 方案概述

GitHub 本身没有"把一个仓库的 Release 自动镜像到另一个仓库"的原生开关，但可以通过 **GitHub Actions** 实现自动化同步：

> 在私有项目里监听 Release 发布事件 → 触发工作流 → 用 `gh` CLI 在公开项目中创建内容、标题、附件都一致的 Release

---

## 前置准备

### 1. 生成 Personal Access Token

跨仓库操作不能用每个仓库自带的默认 `GITHUB_TOKEN`（它只对当前仓库有权限），需要准备一个具备公开项目写权限的令牌。

**推荐使用 Fine-grained PAT：**

1. 进入 GitHub → Settings → Developer settings → Personal access tokens → Fine-grained tokens
2. 点击 "Generate new token"
3. 设置：
   - **Token name**: `sync-release`（或任意名称）
   - **Expiration**: 按需选择
   - **Resource owner**: 选择你的账号
   - **Repository access**: 只选择公开项目
4. 权限设置：
   - `Contents`: Read and write
   - `Actions`: write（如需要）
5. 生成并复制 token

> 💡 使用 Fine-grained PAT 的好处：只授权给公开项目，权限最小化，安全性更高。也可以用 Classic PAT，勾选 `repo` 作用域即可。

### 2. 配置 Secret

在**私有项目**中配置 token：

1. 打开私有仓库 → Settings → Secrets and variables → Actions
2. 点击 "New repository secret"
3. Name 填写：`PUBLIC_REPO_TOKEN`
4. Value 粘贴上面生成的 token
5. 点击 "Add secret"

### 3. 准备公开项目信息

确认公开项目的完整名称，格式为 `用户名/仓库名`，例如 `yourname/your-public-repo`。

---

## Workflow 配置文件

在私有项目的 `.github/workflows/` 目录下新建文件 `sync-release.yml`：

```yaml
name: Sync Release to Public Repo

on:
  release:
    types: [published]

jobs:
  sync:
    runs-on: ubuntu-latest
    steps:
      - name: Sync release
        env:
          GH_TOKEN: ${{ secrets.PUBLIC_REPO_TOKEN }}
          PUBLIC_REPO: yourname/your-public-repo   # 改成你的公开项目全名
          TAG: ${{ github.event.release.tag_name }}
          TITLE: ${{ github.event.release.name }}
          BODY: ${{ github.event.release.body }}
        run: |
          # 1. 下载私有项目当前 Release 的所有附件（仅 assets）
          mkdir -p assets
          gh release download "$TAG" -D assets || true

          # 2. 把 Release notes 写入文件，避免特殊字符导致命令出错
          printf '%s' "$BODY" > notes.md

          # 3. 在公开项目中创建同名 Release
          gh release create "$TAG" \
            --repo "$PUBLIC_REPO" \
            --title "$TITLE" \
            --notes-file notes.md \
            assets/*
```

### 配置说明

| 配置项 | 说明 |
|--------|------|
| `on: release, types: [published]` | 触发条件：每次在私有项目发布新 Release 时自动执行 |
| `secrets.PUBLIC_REPO_TOKEN` | 上一步配置的 PAT，用于跨仓库写入 |
| `gh release download` | 下载私有 Release 的**用户上传附件**（不含 source code 归档） |
| `gh release create` | 在公开项目创建同名 Release，包含标题、正文和附件 |

---

## 关键问题详解

### 编译产物 vs 源码：同步的是什么？

这是最常被问到的问题，需要明确区分两种资源类型：

#### ✅ 会同步的内容：用户上传的 Assets

`gh release download` 默认行为是拉取该 Release 下**由你手动上传的附件**，包括：

- 编译后的安装包（`.exe`, `.dmg`, `.deb`, `.rpm` 等）
- 二进制文件（`.bin`, `.AppImage` 等）
- 压缩包（`.zip`, `.tar.gz` 等，只要是你手动上传的）
- 其他任何作为 asset 上传的文件

这些文件会被原样同步到公开项目的 Release 中。

#### ❌ 不会同步的内容：Source Code 归档

GitHub 自动生成的以下两个下载链接**不会被当作 assets 处理**：

- `Source code (zip)`
- `Source code (tar.gz)`

只有显式加上 `--archive zip\|tar.gz` 参数才会下载它们，默认情况下完全不会出现在同步流程中。

#### 🔍 更精细的控制（可选）

如果只想同步特定类型的文件，可以使用 `--pattern` 参数过滤：

```bash
# 只同步 .zip 和 .AppImage 文件
gh release download "$TAG" -D assets \
  --pattern "*.zip" \
  --pattern "*.AppImage" || true
```

### 关于公开项目 Release 页面的 Source Code 链接

这里有一个重要的平台机制需要理解：

**GitHub 对任何公开仓库的 Release，都会基于该 tag 对应的代码自动显示 Source Code 下载链接，这是平台内置行为，无法关闭。**

但这**不影响你的需求**，原因如下：

1. 公开项目本身就是公开的代码仓库（你的 GitHub Pages 项目）
2. Release 页面显示的 Source Code 链接指向的是**公开项目自己的代码**
3. 公开项目中的代码本来就是对外可见的，不涉及私有内容
4. 你通过 Actions 同步过去的只是编译产物，私有源码不会被带过去

简单来说：**同步过去的是编译产物，页面上显示的 source code 是公开项目自身的公开代码，两者互不干扰。**

### 关于敏感信息

Release 一旦同步到公开项目，notes 正文和附件里的所有内容都会公开。请在发布前确认：

- [ ] Release notes 中没有密钥、内部地址、私有 API 等敏感信息
- [ ] 构建产物中没有嵌入调试信息、内部配置等
- [ ] 附件文件名中不包含敏感信息

### 关于更新和删除的同步（可选）

上面的基础脚本只处理"新建发布"。如果还需要同步编辑和删除操作，可以扩展触发类型：

```yaml
on:
  release:
    types: [published, edited, unpublished]
```

然后在脚本中根据事件类型分别处理：

```bash
case "${{ github.event.action }}" in
  published)
    # 创建新 Release（同上）
    gh release create "$TAG" --repo "$PUBLIC_REPO" ...
    ;;
  edited)
    # 更新已有 Release 的正文
    gh release edit "$TAG" --repo "$PUBLIC_REPO" --notes-file notes.md
    ;;
  unpublished)
    # 删除公开项目中的对应 Release
    gh release delete "$TAG" --repo "$PUBLIC_REPO" --cleanup-tag -y
    ;;
esac
```

这样两边的 Release 状态就能保持完全一致。

### 关于 Tag 的处理

`gh release create` 在目标仓库创建 Release 时：

- 如果同名 tag **已存在**：直接在该 tag 上创建 Release（常见情况，因为公开项目通常与私有项目保持代码同步）
- 如果同名 tag **不存在**：GitHub 会基于目标仓库的默认分支自动创建该 tag

大多数情况下，由于公开项目是私有项目的 Pages 项目，tag 已经存在，可以直接创建 Release。即使不存在也不会报错，只是 tag 会基于公开项目默认分支的最新提交建立。

### 关于费用

- 私有仓库运行 Actions 有免费额度（免费账户每月约 2000 分钟 Linux）
- 这个同步任务每次执行只需十几秒，开销可以忽略不计
- ⚠️ 注意：出于安全考虑，由 `pull_request` 事件触发的 workflow 默认不会读取 secrets，但 `release` 事件不受此限制，token 可以正常使用

---

## 完整流程图

```
┌─────────────────────────────────────────────────────────────┐
│                      私有仓库                                │
│                                                             │
│  开发者发布 Release ──→ 触发 release: published 事件        │
│                           │                                 │
│                           ▼                                 │
│              GitHub Actions 工作流启动                       │
│                           │                                 │
│              ┌────────────┼────────────┐                   │
│              ▼            ▼            ▼                   │
│         下载 Assets    写入 Notes    准备 Tag/Title         │
│              │            │            │                    │
│              └────────────┼────────────┘                    │
│                           ▼                                 │
│              通过 PAT 认证调用公开仓库 API                     │
└───────────────────────────┬─────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────────┐
│                      公开仓库                                │
│                                                             │
│              自动创建同名 Release                             │
│              ├── 标题（title）                               │
│              ├── 正文（notes）                               │
│              ├── 标签（tag）                                 │
│              └── 附件（assets / 编译产物）                    │
│                                                             │
│              用户可直接从公开仓库下载                          │
└─────────────────────────────────────────────────────────────┘
```

---

## 快速上手清单

按照以下步骤操作，即可完成配置：

- [ ] 1. 生成 Fine-grained Personal Access Token（授权给公开项目）
- [ ] 2. 在私有仓库 Settings → Secrets 中添加 `PUBLIC_REPO_TOKEN`
- [ ] 3. 在私有仓库创建 `.github/workflows/sync-release.yml`
- [ ] 4. 修改 `PUBLIC_REPO` 为你的公开项目全名
- [ ] 5. 提交并推送该 workflow 文件到私有仓库
- [ ] 6. 在私有仓库发布一个测试 Release，验证公开仓库是否自动出现对应的 Release
- [ ] 7. （可选）扩展触发类型以支持编辑和删除操作的同步

---

## 参考链接

- [GitHub Actions 事件触发文档](https://docs.github.com/en/actions/using-workflows/events-that-trigger-workflows#release)
- [gh release create 命令手册](https://cli.github.com/manual/gh_release_create)
- [gh release download 命令手册](https://cli.github.com/manual/gh_release_download)
- [Fine-grained Personal Access Tokens 文档](https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/managing-your-personal-access-tokens#fine-grained-personal-access-tokens)

---

*本文档基于 GitHub 平台 2026 年功能编写，如后续 API 或行为变更请以官方文档为准。*
