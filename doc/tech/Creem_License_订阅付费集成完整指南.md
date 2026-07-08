# Creem License 与订阅付费集成完整指南

> 本文档基于 Creem 官方文档（https://docs.creem.io）整理，涵盖 License Key 系统、SDK 选择、自有系统对接、订阅模式设计、多付费模式支持、测试环境接入等全链路集成方案。

---

## 目录

1. [Creem 平台概述](#1-creem-平台概述)
2. [License Key 系统基础](#2-license-key-系统基础)
3. [SDK vs 直接调用 API：如何选择](#3-sdk-vs-直接调用api如何选择)
4. [与自有 License 管理系统对接](#4-与自有-license-管理系统对接)
5. [按年付费 + 过期后不锁死的实现方案](#5-按年付费--过期后不锁死的实现方案)
6. [App 更新分发机制](#6-app-更新分发机制)
7. [多付费模式支持：年付订阅 + 一次性买断](#7-多付费模式支持年付订阅--一次性买断)
8. [测试环境接入流程](#8-测试环境接入流程)
9. [生产环境切换检查清单](#9-生产环境切换检查清单)
10. [安全实践与最佳实践](#10-安全实践与最佳实践)
11. [附录：API 速查表](#11-附录api-速查表)

---

## 1. Creem 平台概述

### 1.1 什么是 Creem？

Creem 是一个 **Merchant of Record（销售记录商）** 平台，专为全球销售的软件开发者设计。作为你的法律销售方，Creem 负责：

- ✅ 支付处理（信用卡、PayPal、Apple Pay、Google Pay 等）
- ✅ 全球税务合规（190+ 国家/地区的 VAT、GST、销售税）
- ✅ 订阅生命周期管理（续费、试用、升级、降级、取消）
- ✅ License Key 自动生成与验证
- ✅ 联盟营销与收益分成
- ✅ Webhook 事件通知

### 1.2 核心优势

| 特性 | 说明 |
|------|------|
| **费率** | 3.9% + 40¢/笔，无月费，无隐藏费用 |
| **税务覆盖** | 190+ 国家，28+ 美国州，欧盟 OSS，英国，韩国 |
| **开发者友好** | TypeScript SDK、Next.js Adapter、CLI 工具 |
| **AI 原生** | 支持 AI Agent 自主接入和管理（SKILL.md） |

### 1.3 快速开始

```bash
# 1. 注册账号：https://creem.io （无需信用卡）

# 2. 获取 API Key
# Dashboard → API Keys → 复制 test key（前缀 creem_test_）

# 3. 安装 SDK
npm install creem

# 4. 初始化
import { Creem } from 'creem'
const creem = new Creem({
  apiKey: 'creem_test_...',
  server: 'test',  // 测试环境用 'test'，生产环境省略或用 'live'
})
```

---

## 2. License Key 系统基础

### 2.1 License Key 生命周期

```
购买 → 自动生成 → 激活(activate) → 定期验证(validate) → 停用(deactivate)
         ↑                                    |
         └───────────── 续费/重新激活 ←────────┘
```

### 2.2 核心 API 端点

#### 激活 License（Activate）

用户首次使用时调用，将 License Key 与设备绑定。

```typescript
// POST /v1/licenses/activate
const activation = await creem.licenses.activate({
  key: 'LICENSE_KEY_HERE',        // 用户输入的 License Key
  metadata: {
    deviceId: 'unique-device-id', // 设备唯一标识
    platform: 'macos',            // 可选：平台信息
    appVersion: '1.0.0',          // 可选：app 版本
  }
})

// 成功返回
{
  id: 'li_xxxxx',
  key: 'LICENSE_KEY_HERE',
  status: 'active',
  activatedAt: '2025-01-01T00:00:00Z',
  expiresAt: '2026-01-01T00:00:00Z',  // 订阅到期时间
  metadata: { deviceId: '...', ... }
}
```

#### 验证 License（Validate）

App 启动或需要检查授权时调用。

```typescript
// POST /v1/licenses/validate
const validation = await creem.licenses.validate({
  key: 'LICENSE_KEY_HERE',
  metadata: {
    deviceId: 'unique-device-id',
  }
})

// 返回状态
{
  id: 'li_xxxxx',
  status: 'active',              // active | expired | suspended | deactivated
  expiresAt: '2026-01-01T00:00:00Z',
  subscriptionStatus: 'active',   // 底层订阅状态
}
```

**关键状态值说明：**

| status | 含义 | App 应对策略 |
|--------|------|-------------|
| `active` | 有效且未过期 | 正常使用所有功能 |
| `expired` | 已过期 | 根据"不锁死"策略处理（见第 5 章） |
| `suspended` | 被暂停 | 禁止使用，提示联系客服 |
| `deactivated` | 已停用 | 需要重新激活 |

#### 停用 License（Deactivate）

用户主动卸载或迁移设备时调用。

```typescript
// POST /v1/licenses/deactivate
const deactivation = await creem.licenses.deactivate({
  key: 'LICENSE_KEY_HERE',
  metadata: {
    deviceId: 'unique-device-id',
  }
})
```

### 2.3 产品配置中的 License 设置

在 Creem Dashboard 创建产品时：

1. 进入 **Products → Create Product**
2. 开启 **Generate license keys on purchase**
3. 配置 License 参数：
   - **Key prefix**: 可选前缀（如 `APP-`）
   - **Key length**: 默认 32 字符
   - **Activation limit**: 允许同时激活的设备数（如 3 台）
   - **Expiration behavior**: 选择过期后行为

---

## 3. SDK vs 直接调用 API：如何选择

### 3.1 对比总览

| 维度 | TypeScript SDK (`creem`) | 直接 HTTP API 调用 |
|------|--------------------------|-------------------|
| **适用场景** | Node.js/Bun/Den 后端服务 | 非 JS 环境（Python、Go、Swift） |
| **类型安全** | ✅ 完整 TypeScript 类型 | ❌ 需自行定义类型 |
| **签名验证** | ✅ 内置 webhook 签名验证 | ⚠️ 需自行实现 HMAC-SHA256 |
| **错误处理** | ✅ 结构化错误对象 | ⚠️ 需自行解析 HTTP 状态码 |
| **依赖** | 需要 npm 包 | 仅需 HTTP 客户端 |
| **包体积** | ~50KB | 无额外依赖 |

### 3.2 推荐决策树

```
你的 App 技术栈是？
├── Node.js / Next.js / Bun / Deno 后端
│   └── ✅ 推荐：TypeScript SDK (npm install creem)
│       import { Creem } from 'creem'
│
├── Python 后端
│   └── ✅ 推荐：直接调用 REST API + requests/httpx
│       使用 requests 库发送 HTTP 请求
│
├── Go 后端
│   └── ✅ 推荐：直接调用 REST API + net/http
│       使用标准库 net/http 发送请求
│
├── Swift/Kotlin (移动端/iOS/Android)
│   └── ✅ 推荐：通过你自己的后端中转
│       移动端 → 你的后端 → Creem API
│       （不要在客户端硬编码 API Key）
│
└── Electron / Tauri (桌面应用)
    └── ✅ 推荐：
        方案 A：主进程内嵌 Node.js → 用 SDK
        方案 B：通过后端 API 中转 → 调用你的 /api/license/* 接口
```

### 3.3 各语言示例

#### Python 示例（直接调用 API）

```python
import requests
import hmac
import hashlib
import time

CREEM_API_BASE = "https://api.creem.io"
API_KEY = "creem_live_..."

def validate_license(key: str, device_id: str) -> dict:
    """验证 License Key"""
    resp = requests.post(
        f"{CREEM_API_BASE}/v1/licenses/validate",
        headers={
            "Authorization": f"Bearer {API_KEY}",
            "Content-Type": "application/json",
        },
        json={
            "key": key,
            "metadata": {"deviceId": device_id}
        }
    )
    resp.raise_for_status()
    return resp.json()

def verify_webhook_signature(payload: bytes, signature: str, secret: str) -> bool:
    """验证 Webhook 签名"""
    expected = hmac.new(
        secret.encode(), payload, hashlib.sha256
    ).hexdigest()
    return hmac.compare_digest(f"sha256={expected}", signature)
```

#### Go 示例（直接调用 API）

```go
package creem

import (
    "bytes"
    "encoding/json"
    "fmt"
    "io"
    "net/http"
)

const BaseURL = "https://api.creem.io"

type Client struct {
    APIKey string
    Client *http.Client
}

func NewClient(apiKey string) *Client {
    return &Client{
        APIKey: apiKey,
        Client: &http.Client{},
    }
}

func (c *Client) ValidateLicense(key, deviceID string) (*LicenseResponse, error) {
    body, _ := json.Marshal(map[string]interface{}{
        "key": key,
        "metadata": map[string]string{"deviceId": deviceID},
    })
    
    req, _ := http.NewRequest("POST", BaseURL+"/v1/licenses/validate", bytes.NewReader(body))
    req.Header.Set("Authorization", "Bearer "+c.APIKey)
    req.Header.Set("Content-Type", "application/json")
    
    resp, err := c.Client.Do(req)
    if err != nil {
        return nil, err
    }
    defer resp.Body.Close()
    
    result := &LicenseResponse{}
    json.NewDecoder(io.LimitReader(resp.Body, 1<<20)).Decode(result)
    return result, nil
}
```

### 3.4 架构建议：统一后端代理

无论前端是什么技术栈，**强烈建议通过你自己的后端统一代理 Creem API 调用**：

```
┌─────────────┐     ┌──────────────┐     ┌──────────┐
│  Desktop App │────▶│ Your Backend │────▶│  Creem   │
│  Mobile App  │────▶│ /api/license │     │   API    │
│  Web App     │────▶│ /api/license │     │          │
└─────────────┘     └──────────────┘     └──────────┘
                          │
                          ▼
                   ┌──────────────┐
                   │  Your DB     │
                   │ (缓存授权状态) │
                   └──────────────┘
```

**好处：**
- API Key 不暴露给客户端
- 可以缓存授权状态，减少对 Creem 的请求
- 可以添加自定义业务逻辑（如功能开关、试用期管理）
- 统一日志和监控

---

## 4. 与自有 License 管理系统对接

### 4.1 两种对接模式

#### 模式 A：Creem 作为权威数据源（推荐）

Creem 管理 License 的生成、状态变更；你的系统只做缓存和同步。

```
                    ┌─────────────────────────────────────┐
                    │           数据流向                  │
                    ├─────────────────────────────────────┤
                    │                                     │
   用户购买  ──▶  Creem  ──▶  Webhook  ──▶  你的后端      │
                     生成               同步到你的 DB     │
                     License                              │
                        │                                 │
   App 验证  ──▶  你的后端  ──▶  缓存命中？                │
                     │              │                     │
                     │ Yes          │ No                  │
                     ▼              ▼                     │
                  返回缓存      调 Creem API 验证           │
                                并更新缓存                 │
                    │                                     │
                    └─────────────────────────────────────┘
```

**实现要点：**

```typescript
// 你的后端：接收 Creem Webhook
app.post('/api/webhooks/creem', async (req, res) => {
  // 1. 验证签名
  const signature = req.headers['creem-signature']
  const isValid = verifyWebhookSignature(req.body, signature, WEBHOOK_SECRET)
  if (!isValid) return res.status(401).send('Invalid signature')

  // 2. 根据事件类型处理
  const event = req.body
  switch (event.type) {
    case 'license.activated':
      await db.licenses.upsert({
        creemLicenseId: event.data.id,
        key: event.data.key,
        status: 'active',
        expiresAt: new Date(event.data.expiresAt),
        customerId: event.data.customer?.id,
        subscriptionId: event.data.subscription?.id,
        metadata: event.data.metadata,
      })
      break

    case 'subscription.renewed':
      // 更新过期时间
      await db.licenses.update({
        where: { subscriptionId: event.data.id },
        data: { 
          status: 'active',
          expiresAt: new Date(event.data.currentPeriodEnd),
        }
      })
      break

    case 'subscription.expired':
    case 'subscription.canceled':
      await db.licenses.update({
        where: { subscriptionId: event.data.id },
        data: { status: 'expired' },
      })
      break

    case 'license.deactivated':
      await db.licenses.update({
        where: { creemLicenseId: event.data.id },
        data: { status: 'deactivated' },
      })
      break
  }

  res.status(200).send('OK')
})

// 你的后端：提供给 App 的统一接口
app.get('/api/license/status', async (req, res) => {
  const { key, deviceId } = req.query

  // 1. 先查本地缓存
  let license = await db.licenses.findUnique({ where: { key } })

  // 2. 缓存不存在或已过期，调 Creem API 验证
  if (!license || isCacheStale(license)) {
    const validation = await creem.licenses.validate({ key, metadata: { deviceId } })
    license = await db.licenses.upsert({
      where: { key },
      update: {
        status: validation.status,
        expiresAt: new Date(validation.expiresAt),
        lastValidatedAt: new Date(),
      },
      create: {
        key,
        status: validation.status,
        expiresAt: new Date(validation.expiresAt),
        lastValidatedAt: new Date(),
      },
    })
  }

  // 3. 返回统一格式
  res.json({
    valid: license.status === 'active',
    status: license.status,
    expiresAt: license.expiresAt,
    updateEligible: new Date() < license.expiresAt,  // 是否可更新
  })
})
```

#### 模式 B：自有系统作为权威数据源

你的系统管理 License 映射关系；Creem 只负责支付和基础 License 生成。

**适用场景：**
- 你已有成熟的 License 管理系统
- 需要复杂的 License 策略（浮动许可、离线许可等）
- 需要与内部 CRM/ERP 对接

**实现方式：**

```typescript
// 购买成功后的 Webhook 处理
app.post('/api/webhooks/creem', async (req, res) => {
  const event = req.body
  
  if (event.type === 'checkout.completed') {
    // 1. 从 Creem 获取生成的 License Key
    const checkout = await creem.checkouts.retrieve(event.data.id)
    const licenseKey = checkout.licenseKey  // Creem 生成的 key
    
    // 2. 在你的系统中创建映射
    await yourLicenseSystem.createLicense({
      externalKey: licenseKey,       // Creem 的 key
      internalId: generateInternalId(), // 你自己的 ID
      customerEmail: event.data.customer.email,
      plan: mapProductToPlan(checkout.productId), // 映射到你的套餐
      features: getPlanFeatures(checkout.productId),
      expiresAt: calculateExpiry(checkout),
    })
  }
  
  res.status(200).send('OK')
})

// App 验证时，走你自己的系统
app.get('/api/license/check', async (req, res) => {
  const { key } = req.query
  
  // 1. 在你的系统中查找
  const license = await yourLicenseSystem.findByExternalKey(key)
  
  if (!license) {
    // 可能是纯 Creem License（无自定义映射），fallback 到 Creem
    const validation = await creem.licenses.validate({ key })
    return res.json({ source: 'creem', ...validation })
  }
  
  // 2. 返回你的系统的授权信息（包含更多业务逻辑）
  res.json({
    source: 'internal',
    valid: license.isValid(),
    plan: license.plan,
    features: license.features,
    expiresAt: license.expiresAt,
    updateEligible: !license.isExpired(),
  })
})
```

### 4.2 如何选择？

| 因素 | 模式 A（Creem 权威） | 模式 B（自有权威） |
|------|---------------------|-------------------|
| 实现复杂度 | 🟢 低 | 🔴 高 |
| 自定义程度 | 🟡 中等 | 🟢 高 |
| 维护成本 | 🟢 低 | 🔴 高 |
| 数据一致性 | 🟢 高（单一源） | 🟡 需要同步 |
| 适用场景 | 新项目、SaaS MVP | 已有成熟系统、企业级 |

**一般建议：新项目从模式 A 开始，后续有特殊需求再迁移到模式 B。**

---

## 5. 按年付费 + 过期后不锁死的实现方案

### 5.1 业务需求理解

```
✅ 有效期内：正常使用 + 可更新到最新版本
❌ 过期后：仍可继续使用当前版本（不锁死）
❌ 过期后：不能更新到新版本
💡 续费后：恢复全部权限
```

### 5.2 Creem 侧配置

1. **创建年度订阅产品**

```bash
# 通过 CLI
creem products create \
  --name "My App Pro (Annual)" \
  --price 19900 \           # $199.00/年
  --currency USD \
  --billing-type recurring \
  --billing-period every-year \
  --generate-licenses       # 自动生成 License Key
```

或在 Dashboard 中：
- Products → Create Product
- Price: `$199.00/year`
- Billing: Recurring → Annual
- ✅ Enable "Generate license keys on purchase"

2. **配置 License 过期行为**

在产品设置中，确保 License 的 `expiresAt` 与订阅周期同步（这是默认行为）。

### 5.3 核心实现：`updateEligible` 状态

在你的系统中引入一个关键状态字段：

```typescript
interface LicenseState {
  // 基础状态
  status: 'active' | 'expired' | 'suspended' | 'deactivated'
  expiresAt: Date
  
  // 关键字段：是否有资格获取更新
  updateEligible: boolean  // true = 可更新，false = 不可更新
  
  // 元数据
  planType: 'annual' | 'lifetime'
  lastCheckedVersion: string
}
```

### 5.4 Webhook 同步逻辑

```typescript
// 监听订阅相关事件，同步 updateEligible 状态
async function handleSubscriptionEvent(event: CreemWebhookEvent) {
  const subscriptionId = event.data.id
  const license = await db.licenses.findFirst({
    where: { subscriptionId }
  })

  switch (event.type) {
    case 'subscription.paid':
    case 'subscription.renewed':
      // 续费成功 → 恢复更新权限
      await db.licenses.update({
        where: { id: license.id },
        data: {
          status: 'active',
          expiresAt: new Date(event.data.currentPeriodEnd),
          updateEligible: true,   // ✅ 可以更新了
        }
      })
      break

    case 'subscription.expired':
    case 'subscription.past_due':  // 连续扣款失败
      // 过期 → 禁止更新，但不锁死
      await db.licenses.update({
        where: { id: license.id },
        data: {
          status: 'expired',
          updateEligible: false,  // ❌ 不能更新了
          // 注意：不修改其他功能权限
        }
      })
      break

    case 'subscription.canceled':
      // 用户主动取消（但在 currentPeriodEnd 前仍有效）
      // 不立即改变状态，等到期后再处理
      console.log(`Subscription ${subscriptionId} canceled, will expire at ${event.data.currentPeriodEnd}`)
      break

    case 'subscription.resumed':
      // 用户取消了但又恢复了
      await db.licenses.update({
        where: { id: license.id },
        data: {
          status: 'active',
          updateEligible: true,
        }
      })
      break
  }
}
```

### 5.5 App 侧授权检查逻辑

```typescript
// App 启动时的统一检查
async function checkLicenseStatus(): Promise<LicenseState> {
  const response = await fetch('/api/license/status', {
    method: 'POST',
    body: JSON.stringify({
      key: storedLicenseKey,
      deviceId: getDeviceId(),
    }),
  })
  
  const state: LicenseState = await response.json()
  
  // 应用授权逻辑
  applyLicenseState(state)
  
  return state
}

function applyLicenseState(state: LicenseState) {
  const now = new Date()
  
  // 1. 基础功能可用性（不过期不锁死）
  if (state.status === 'active' || state.status === 'expired') {
    // ✅ 核心功能始终可用
    enableCoreFeatures()
  } else if (state.status === 'suspended') {
    // ❌ 被暂停，完全禁用
    disableAllFeatures()
    showSuspensionNotice()
    return
  }
  
  // 2. 更新权限控制
  if (state.updateEligible && now < state.expiresAt) {
    // ✅ 可以检查并下载更新
    enableAutoUpdate()
    checkForUpdates()
  } else {
    // ❌ 禁止自动更新
    disableAutoUpdate()
    
    // 显示续费提示（非强制弹窗）
    if (state.status === 'expired') {
      showRenewalBanner(
        message: "您的订阅已过期，当前版本可继续使用。续费后即可获取最新版本。",
        cta: "续费",
        onCTA: () => openBrowser('https://your-store.creem.io/billing')
      )
    }
  }
}

// 手动检查更新时
async function checkForUpdates(): Promise<UpdateResult> {
  const state = await getCurrentLicenseState()
  
  if (!state.updateEligible) {
    return {
      canUpdate: false,
      reason: 'SUBSCRIPTION_EXPIRED',
      message: '订阅已过期，请续费以获取最新版本。',
    }
  }
  
  // 正常检查更新逻辑
  const latestVersion = await fetchLatestVersion()
  const currentVersion = getAppVersion()
  
  if (isNewer(latestVersion, currentVersion)) {
    return { canUpdate: true, version: latestVersion }
  }
  
  return { canUpdate: false, reason: 'UP_TO_DATE' }
}
```

### 5.6 UI/UX 设计建议

```
┌─────────────────────────────────────────────────────┐
│  My App v1.5.0                        [✓ 已激活]    │
├─────────────────────────────────────────────────────┤
│                                                     │
│  ✅ 许可证状态：有效                                  │
│  ✅ 到期时间：2026-01-01                             │
│  ✅ 更新权限：可用                                   │
│                                                     │
│  当前版本：v1.5.0                                    │
│  最新版本：v1.6.0        [🔄 立即更新]               │
│                                                     │
└─────────────────────────────────────────────────────┘


┌─────────────────────────────────────────────────────┐
│  My App v1.5.0                        [⚠ 已过期]    │
├─────────────────────────────────────────────────────┤
│                                                     │
│  ⚠️ 许可证状态：已过期（2025-06-01 到期）             │
│  ℹ️ 当前版本可继续使用                               │
│  ❌ 更新权限：不可用                                 │
│                                                     │
│  最新版本：v2.0.0                                    │
│  [🔒 需要续费才能获取此更新]                         │
│                                                     │
│  ┌─────────────────────────────────────┐            │
│  │  💳 续费订阅 — $199/年              │            │
│  │     [前往续费]                      │            │
│  └─────────────────────────────────────┘            │
│                                                     │
└─────────────────────────────────────────────────────┘
```

---

## 6. App 更新分发机制

### 6.1 重要说明：Creem 不负责 App 分发

**Creem 是支付和 License 管理平台，不是应用分发平台。** 你需要自己搭建或选择更新分发服务。

### 6.2 推荐架构

```
┌────────────────────────────────────────────────────────────┐
│                      更新分发架构                           │
├────────────────────────────────────────────────────────────┤
│                                                            │
│   ┌─────────┐                                             │
│   │  你的    │  构建 → 上传                                │
│   │  CI/CD   ───────────────────────────────────┐         │
│   └─────────┘                                   │         │
│                                                 ▼         │
│   ┌─────────────────┐   ┌───────────────────┐  ┌────────┐ │
│   │  更新服务器      │   │  版本元数据 API   │  │ CDN/   │ │
│   │  (GitHub Releases│◀──│  /api/version    │  │ 对象存储│ │
│   │   或自建)        │──▶│  返回最新版本号   │  │ S3/OSS │ │
│   └─────────────────┘   └───────────────────┘  └────────┘ │
│          ▲                       ▲                  ▲     │
│          │                       │                  │     │
│          │ 下载安装包            │ 检查更新          │ 托管 │
│          │                       │                  │ 文件 │
│          │                       │                  │     │
│   ┌──────┴──────┐        ┌──────┴──────┐            │     │
│   │  Desktop App │────────▶  Update    │────────────┘     │
│   │  Mobile App  │         Checker                       │
│   └─────────────┘         └─────────────────────────────┘ │
│                                                            │
│   授权检查（每次更新前）：                                   │
│   App → /api/license/status → updateEligible? → 决定是否更新│
│                                                            │
└────────────────────────────────────────────────────────────┘
```

### 6.3 更新服务器选项

| 方案 | 适用场景 | 复杂度 | 成本 |
|------|---------|--------|------|
| **GitHub Releases** | 开源/闭源工具 | 🟢 低 | 免费（私有仓库需付费） |
| **自建 S3/Cloudflare R2** | 商业软件 | 🟡 中 | 低（存储+流量费） |
| **Electron Updater** | Electron App | 🟢 低 | 免费 |
| **Sparkle (macOS)** | 原生 macOS App | 🟡 中 | 需要服务器 |
| **Google Play / App Store** | 移动端 | 🟢 低 | 平台费用 |

### 6.4 更新检查示例代码

```typescript
// 后端：版本检查 API
app.get/api/version/latest', async (req, res) => {
  // 返回最新版本信息（不含下载 URL，授权后在下一步提供）
  res.json({
    version: '2.0.0',
    releaseDate: '2025-07-01',
    changelog: [
      '- 新增：AI 助手功能',
      '- 优化：启动速度提升 50%',
      '- 修复：内存泄漏问题',
    ],
    minRequiredVersion: '1.5.0',  // 最低兼容版本
    critical: false,               // 是否为安全修复
  })
})

// 后端：获取下载链接（需要授权）
app.post('/api/version/download', async (req, res) => {
  const { currentVersion, deviceInfo } = req.body
  const licenseKey = req.headers['x-license-key']
  
  // 1. 验证 License
  const license = await validateLicense(licenseKey, deviceInfo.deviceId)
  
  if (!license.updateEligible) {
    return res.status(403).json({
      error: 'SUBSCRIPTION_EXPIRED',
      message: '订阅已过期，无法下载更新。请续费后重试。',
      renewUrl: 'https://your-store.creem.io/billing',
    })
  }
  
  // 2. 生成带签名的临时下载 URL（防泄露）
  const downloadUrl = await generateSignedDownloadUrl({
    version: '2.0.0',
    platform: deviceInfo.platform,
    expiresIn: 3600,  // 1 小时有效
  })
  
  res.json({ downloadUrl })
})
```

### 6.5 Electron App 更新示例

```typescript
// electron/main.ts (使用 electron-updater)
import { autoUpdater } from 'electron-updater'
import { getLicenseState } from './license'

async function setupAutoUpdater() {
  autoUpdater.setFeedURL({
    provider: 'generic',
    url: 'https://updates.your-app.com',
  })
  
  // 每次检查更新前，先验证授权
  autoUpdater.on('before-check-for-update', async () => {
    const state = await getLicenseState()
    if (!state.updateEligible) {
      // 取消本次更新检查
      autoUpdater.emit('update-not-available', {
        version: app.getVersion(),
      })
      
      // 提示用户续费
      mainWindow.webContents.send('update:blocked', {
        reason: 'subscription_expired',
        message: '订阅已过期，无法获取更新。',
      })
    }
  })
  
  autoUpdater.on('update-available', (info) => {
    mainWindow.webContents.send('update:available', info)
  })
  
  autoUpdater.on('download-progress', (progress) => {
    mainWindow.webContents.send('update:progress', progress)
  })
  
  autoUpdater.on('update-downloaded', (info) => {
    mainWindow.webContents.send('update:ready', info)
  })
}
```

---

## 7. 多付费模式支持：年付订阅 + 一次性买断

### 7.1 产品配置方案

在 Creem 中创建两个独立产品：

| 产品 | 类型 | 价格 | License 行为 |
|------|------|------|-------------|
| **Pro Annual** | 年付订阅 | $199/年 | `expiresAt` 随订阅周期更新 |
| **Pro Lifetime** | 一次性买断 | $499 一次性 | `expiresAt = null`（永不过期） |

```bash
# 创建年付订阅产品
creem products create \
  --name "My App Pro (Annual)" \
  --price 19900 \
  --currency USD \
  --billing-type recurring \
  --billing-period every-year \
  --generate-licenses \
  --description "年付订阅，持续获取更新和技术支持"

# 创建一次性买断产品
creem products create \
  --name "My App Pro (Lifetime)" \
  --price 49900 \
  --currency USD \
  --billing-type one_time \
  --generate-licenses \
  --description "一次性买断，永久使用（含 1 年更新）"
```

### 7.2 License 类型区分

在数据库 schema 中增加 `planType` 字段：

```sql
CREATE TABLE licenses (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  creem_license_id VARCHAR(64) UNIQUE NOT NULL,
  key VARCHAR(128) UNIQUE NOT NULL,
  
  -- 授权类型
  plan_type VARCHAR(20) NOT NULL DEFAULT 'annual',  -- 'annual' | 'lifetime'
  
  -- 状态
  status VARCHAR(20) NOT NULL DEFAULT 'active',
  expires_at TIMESTAMP WITH TIME ZONE,  -- lifetime 为 NULL
  update_eligible BOOLEAN NOT NULL DEFAULT true,
  
  -- 关联
  customer_id VARCHAR(64),
  subscription_id VARCHAR(64),  -- lifetime 为 NULL
  product_id VARCHAR(64) NOT NULL,
  
  -- 时间戳
  created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
  updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
  last_validated_at TIMESTAMP WITH TIME ZONE
);
```

### 7.3 统一授权接口

```typescript
// GET /api/license/status
app.post('/api/license/status', async (req, res) => {
  const { key, deviceId } = req.body
  
  // 1. 查找 License
  let license = await db.licenses.findUnique({ where: { key } })
  
  // 2. 如果本地没有，调 Creem API
  if (!license) {
    const result = await creem.licenses.validate({ key, metadata: { deviceId } })
    
    // 根据 product_id 判断类型
    const product = await creem.products.retrieve(result.productId)
    const planType = product.billingType === 'one_time' ? 'lifetime' : 'annual'
    
    license = await db.licenses.create({
      data: {
        creemLicenseId: result.id,
        key: result.key,
        planType,
        status: result.status,
        expiresAt: result.expiresAt ? new Date(result.expiresAt) : null,
        updateEligible: result.status === 'active',
        productId: result.productId,
        subscriptionId: result.subscriptionId,
        lastValidatedAt: new Date(),
      }
    })
  }
  
  // 3. 计算最终状态
  const now = new Date()
  let finalUpdateEligible: boolean
  
  if (license.planType === 'lifetime') {
    // 买断用户：永不过期，永远可以更新
    finalUpdateEligible = true
  } else {
    // 订阅用户：根据有效期判断
    finalUpdateEligible = license.status === 'active' && 
                          license.expiresAt && 
                          now < license.expiresAt
  }
  
  // 4. 返回统一格式
  res.json({
    valid: license.status === 'active' || license.status === 'expired',
    status: license.status,
    planType: license.planType,        // 'annual' | 'lifetime'
    expiresAt: license.expiresAt,       // lifetime 为 null
    updateEligible: finalUpdateEligible,
    
    // 友好显示信息
    displayInfo: {
      planName: license.planType === 'lifetime' ? '专业版（永久授权）' : '专业版（年付订阅）',
      expiryMessage: license.planType === 'lifetime' 
        ? '永久授权' 
        : license.expiresAt 
          ? `到期于 ${formatDate(license.expiresAt)}`
          : '未知',
    }
  })
})
```

### 7.4 App 侧处理双模式

```typescript
interface LicenseResponse {
  valid: boolean
  status: string
  planType: 'annual' | 'lifetime'
  expiresAt: string | null
  updateEligible: boolean
  displayInfo: {
    planName: string
    expiryMessage: string
  }
}

function applyLicense(state: LicenseResponse) {
  // 1. 基础功能（两种模式都一样）
  if (state.valid && state.status !== 'suspended') {
    enableCoreFeatures()
  }
  
  // 2. 更新权限（根据类型区分）
  if (state.updateEligible) {
    enableAutoUpdate()
    checkForUpdates()
  } else {
    disableAutoUpdate()
    
    if (state.planType === 'annual') {
      // 订阅用户：提示续费
      showRenewalPrompt({
        title: '订阅已过期',
        message: `${state.displayInfo.expiryMessage}。续费后即可获取最新版本。`,
        ctaText: '续费 ($199/年)',
        ctaUrl: 'https://your-store.creem.io/billing',
        upgradeUrl: 'https://your-store.creem.io/checkout/lifetime',  // 升级到买断
      })
    }
    // lifetime 用户不会走到这里（updateEligible 永远为 true）
  }
  
  // 3. UI 显示
  updateLicenseUI({
    planName: state.displayInfo.planName,
    expiryMessage: state.displayInfo.expiryMessage,
    status: state.status,
  })
}
```

### 7.5 Storefront 展示两个选项

在你的 Creem Storefront 或自建落地页中，清晰展示两种模式：

```
┌─────────────────────────────────────────────────────────────┐
│                     Choose Your Plan                        │
├────────────────────────┬────────────────────────────────────┤
│                        │                                    │
│   📅 Pro Annual        │   👑 Pro Lifetime                 │
│                        │                                    │
│   $199/year            │   $499  once                       │
│   Save $300 vs monthly │   Pay once, use forever            │
│                        │                                    │
│   ✓ All features       │   ✓ All features                   │
│   ✓ Updates & support  │   ✓ Lifetime updates*             │
│   ✓ Priority support   │   ✓ Priority support               │
│                        │                                    │
│   [Subscribe Annually] │   [Buy Lifetime]                   │
│                        │                                    │
│                        │   *Includes 1 year of updates,     │
│                        │     renewable optionally           │
│                        │                                    │
└────────────────────────┴────────────────────────────────────┘
```

---

## 8. 测试环境接入流程

### 8.1 Creem 测试环境概述

| 项目 | 测试环境 | 生产环境 |
|------|---------|---------|
| **API 地址** | `https://test-api.creem.io` | `https://api.creem.io` |
| **API Key 前缀** | `creem_test_...` | `creem_...` |
| **Dashboard** | `https://dashboard.creem.io` (Test Mode) | 同上（Live Mode） |
| **支付** | 模拟支付（测试卡号） | 真实支付 |
| **Webhooks** | 需要公开可达的 URL | 同左 |

### 8.2 六步接入流程

#### Step 1：获取测试 API Key

```bash
# 1. 登录 https://dashboard.creem.io
# 2. 进入 Settings → API Keys
# 3. 点击 "Create Test Key"
# 4. 复制以 creem_test_ 开头的 Key

export CREEM_TEST_API_KEY="creem_test_your_key_here"
```

#### Step 2：初始化测试环境 SDK

```typescript
// config/creem.test.ts
import { Creem } from 'creem'

export const creemTest = new Creem({
  apiKey: process.env.CREEM_TEST_API_KEY!,
  server: 'test',  // 关键：指定测试环境
})

// 验证连接
async function testConnection() {
  try {
    const products = await creemTest.products.list()
    console.log('✅ Test environment connected. Found', products.length, 'products')
  } catch (error) {
    console.error('❌ Failed to connect to test environment:', error.message)
  }
}
```

#### Step 3：创建测试产品

```bash
# 通过 CLI 创建测试产品
creem --api-key $CREEM_TEST_API_KEY products create \
  --name "My App Pro (Test)" \
  --price 1999 \
  --currency USD \
  --billing-type recurring \
  --billing-period every-month \  # 测试用月付，方便快速验证周期
  --generate-licenses
```

或通过 API：

```typescript
const testProduct = await creemTest.products.create({
  name: 'My App Pro (Test)',
  prices: [{
    amount: 1999,  // $19.99 测试价格
    currency: 'usd',
    billingType: 'recurring',
    billingPeriod: 'every-month',
  }],
  generateLicenses: true,
})

console.log('Test product created:', testProduct.id)
```

#### Step 4：模拟支付流程

**测试卡号：**

| 卡号 | 场景 | 结果 |
|------|------|------|
| `4242 4242 4242 4242` | 正常支付 | ✅ 成功 |
| `4000 0000 0000 0002` | 卡被拒绝 | ❌ 失败 |
| `4000 0000 0000 0069` | 过期卡 | ❌ 失败 |
| `4000 0000 0000 0127` | 余额不足 | ❌ 失败 |
| `4000 0000 0000 2037` | 需要验证 | ⚠️ 3D Secure |

**任意有效的未来日期**（如 `12/30`）和 **任意 CVC**（如 `123`）均可。

```typescript
// 创建测试 Checkout 链接
const checkout = await creemTest.checkouts.create({
  productId: testProduct.id,
  successUrl: 'http://localhost:3000/success?session_id={CHECKOUT_SESSION_ID}',
  cancelUrl: 'http://localhost:3000/cancel',
})

console.log('Test checkout URL:', checkout.checkoutUrl)
// 在浏览器打开此 URL，使用测试卡号完成支付
```

#### Step 5：本地调试 Webhooks

开发环境的 localhost 无法接收 Creem Webhook 回调。解决方案：

**方案 A：ngrok（推荐）**

```bash
# 1. 安装 ngrok
brew install ngrok  # 或从 https://ngrok.com 下载

# 2. 启动隧道
ngrok http 3000
# 输出：Forwarding    https://xxxx.ngrok-free.app -> http://localhost:3000

# 3. 在 Creem Dashboard 配置 Webhook URL
# Settings → Webhooks → Add endpoint
# URL: https://xxxx.ngrok-free.app/api/webhooks/creem
```

**方案 B：Creem CLI 本地转发**

```bash
# Creem CLI 内置 webhook 转发功能
creem webhooks forward --port 3000
# 会自动创建隧道并将 webhook 转发到本地
```

**Webhook 验证示例：**

```typescript
app.post('/api/webhooks/creem', async (req, res) => {
  const signature = req.headers['creem-signature'] as string
  
  // 验证签名（测试和生产都要验证！）
  const isValid = await creemTest.webhooks.verifySignature({
    payload: JSON.stringify(req.body),
    signature,
  })
  
  if (!isValid) {
    console.warn('⚠️ Invalid webhook signature')
    return res.status(401).send('Invalid signature')
  }
  
  // 开发环境：打印完整事件便于调试
  console.log('📦 Webhook received:', {
    type: req.body.type,
    id: req.body.id,
    data: JSON.stringify(req.body.data, null, 2),
  })
  
  // 处理事件...
  await handleWebhookEvent(req.body)
  
  res.status(200).send('OK')
})
```

#### Step 6：编写自动化测试

```typescript
// __tests__/license.test.ts
import { describe, it, expect, beforeAll } from '@jest/globals'
import { creemTest } from '../config/creem.test'

describe('License API (Test Environment)', () => {
  let testLicenseKey: string
  
  beforeAll(async () => {
    // 创建测试 Checkout 并获取 License Key
    const checkout = await creemTest.checkouts.create({
      productId: process.env.TEST_PRODUCT_ID!,
      successUrl: 'http://localhost',
    })
    // 注意：实际测试中需要模拟支付完成后的回调
    // 这里假设已经有一个测试用的 License Key
    testLicenseKey = process.env.TEST_LICENSE_KEY!
  })
  
  describe('POST /v1/licenses/activate', () => {
    it('should activate a valid license', async () => {
      const result = await creemTest.licenses.activate({
        key: testLicenseKey,
        metadata: { deviceId: 'test-device-001' },
      })
      
      expect(result.status).toBe('active')
      expect(result.id).toBeDefined()
      expect(result.activatedAt).toBeDefined()
    })
    
    it('should reject duplicate activation for same device', async () => {
      // 同一设备重复激活应返回已有激活信息
      const result = await creemTest.licenses.activate({
        key: testLicenseKey,
        metadata: { deviceId: 'test-device-001' },
      })
      
      expect(result.status).toBe('active')
    })
    
    it('should reject invalid license key', async () => {
      await expect(
        creemTest.licenses.activate({
          key: 'INVALID_KEY',
          metadata: { deviceId: 'test-device-002' },
        })
      ).rejects.toThrow()
    })
  })
  
  describe('POST /v1/licenses/validate', () => {
    it('should return active status for valid license', async () => {
      const result = await creemTest.licenses.validate({
        key: testLicenseKey,
        metadata: { deviceId: 'test-device-001' },
      })
      
      expect(result.status).toBe('active')
    })
    
    it('should fail for deactivated license', async () => {
      // 先停用
      await creemTest.licenses.deactivate({
        key: testLicenseKey,
        metadata: { deviceId: 'test-device-001' },
      })
      
      const result = await creemTest.licenses.validate({
        key: testLicenseKey,
        metadata: { deviceId: 'test-device-001' },
      })
      
      expect(result.status).toBe('deactivated')
    })
  })
})
```

### 8.3 测试场景清单

| 测试场景 | 操作 | 预期结果 |
|---------|------|---------|
| 首次激活 | `activate` 有效 key | status=active |
| 重复激活 | 同设备再次 activate | 返回已有激活 |
| 超限激活 | 超过 maxActivations 设备数 | 报错 |
| 正常验证 | `validate` 有效 key | 返回正确状态 |
| 过期验证 | 订阅过期后 validate | status=expired |
| 停用后验证 | deactivate 后 validate | status=deactivated |
| 支付成功 | Checkout + 测试卡 4242... | 生成 License |
| 支付失败 | Checkout + 测试卡 4000... | 支付被拒 |
| 续费 Webhook | subscription.renewed | updateEligible=true |
| 过期 Webhook | subscription.expired | updateEligible=false |

---

## 9. 生产环境切换检查清单

### 9.1 切换前准备

- [ ] **KYC 审核**
  - [ ] 完成 Creem Dashboard 中的身份验证
  - [ ] 提交营业执照/个人身份证
  - [ ] 等待审核通过（通常 1-2 个工作日）

- [ ] **API Key**
  - [ ] 创建生产 API Key（前缀 `creem_...`）
  - [ ] 将 Key 存储到密钥管理服务（如 AWS Secrets Manager、Vault）
  - [ ] **切勿**硬编码在代码中或提交到 Git

- [ ] **产品配置**
  - [ ] 在生产环境创建正式产品
  - [ ] 设置正确的价格和计费周期
  - [ ] 开启 License Key 生成
  - [ ] 配置正确的过期行为

- [ ] **Webhook**
  - [ ] 配置生产环境的 Webhook URL（必须是 HTTPS）
  - [ ] 确保签名验证逻辑已启用
  - [ ] 测试 Webhook 端点可达性

- [ ] **域名与品牌**
  - [ ] 配置自定义域名（可选）
  - [ ] 上传 Logo 和品牌素材
  - [ ] 设置 Customer Portal 域名

### 9.2 配置切换

```typescript
// config/creem.ts
import { Creem } from 'creem'

const creem = new Creem({
  apiKey: process.env.CREEM_API_KEY!,  // 生产 key
  // 生产环境不需要 server: 'test'，默认就是 live
  // 或者显式指定：server: 'live'
})

export default creem
```

```bash
# .env.production
CREEM_API_KEY=creem_live_your_production_key
CREEM_WEBHOOK_SECRET=whsec_your_webhook_secret
```

### 9.3 切换后验证

- [ ] 用真实小额支付测试完整流程
- [ ] 验证 License 激活/验证/停用
- [ ] 验证 Webhook 事件接收和处理
- [ ] 验证订阅续费逻辑
- [ ] 验证过期后"不锁死"行为
- [ ] 检查日志和监控告警

---

## 10. 安全实践与最佳实践

### 10.1 API Key 安全

```bash
# ✅ 正确做法：环境变量
export CREEM_API_KEY="creem_live_..."

# ❌ 错误做法：硬编码
const apiKey = "creem_live_..."  # 永远不要这样做！

# ❌ 错误做法：提交到版本控制
# 确保 .gitignore 包含：
# .env
# *.env.local
# credentials.json
```

### 10.2 Webhook 签名验证

**每次收到 Webhook 都必须验证签名！**

```typescript
import crypto from 'crypto'

export function verifyWebhookSignature(
  payload: string | object,
  signature: string,
  secret: string
): boolean {
  const payloadStr = typeof payload === 'string' ? payload : JSON.stringify(payload)
  
  const expectedSignature = crypto
    .createHmac('sha256', secret)
    .update(payloadStr)
    .digest('hex')
  
  const expected = `sha256=${expectedSignature}`
  
  return crypto.timingSafeEqual(
    Buffer.from(signature),
    Buffer.from(expected)
  )
}
```

### 10.3 错误处理与日志

```typescript
class CreemIntegrationError extends Error {
  constructor(
    operation: string,
    public statusCode?: number,
    public creemError?: unknown
  ) {
    super(`Creem ${operation} failed`)
    this.name = 'CreemIntegrationError'
  }
}

async function safeLicenseValidate(key: string, deviceId: string) {
  try {
    return await creem.licenses.validate({ key, metadata: { deviceId } })
  } catch (error) {
    // 记录完整错误用于排查
    logger.error('License validation failed', {
      key: maskLicenseKey(key),  // 不要记录完整 key
      deviceId,
      error: error instanceof Error ? error.message : 'Unknown error',
      statusCode: error?.status,
      creemCode: error?.code,
    })
    
    // 根据错误类型决定是否放行
    if (isNetworkError(error)) {
      // 网络问题：允许使用缓存的授权状态（如果有的话）
      return getCachedLicenseState(key)
    }
    
    if (error?.status === 404) {
      // License 不存在
      throw new CreemIntegrationError('validate', 404, error)
    }
    
    if (error?.status === 401 || error?.status === 403) {
      // 认证问题：立即告警
      alertTeam('Creem API authentication failed!')
      throw new CreemIntegrationError('validate', 403, error)
    }
    
    // 其他错误：保守处理
    throw new CreemIntegrationError('validate', undefined, error)
  }
}
```

### 10.4 性能优化

```typescript
// 缓存 License 验证结果
const licenseCache = new Map<string, { data: LicenseState; ttl: number }>()

async function getCachedOrValidate(key: string, deviceId: string): Promise<LicenseState> {
  const cacheKey = `${key}:${deviceId}`
  const cached = licenseCache.get(cacheKey)
  
  // 缓存有效期内，直接返回
  if (cached && Date.now() < cached.ttl) {
    return cached.data
  }
  
  // 调用 API 并缓存结果（缓存 5 分钟）
  const result = await safeLicenseValidate(key, deviceId)
  licenseCache.set(cacheKey, {
    data: result,
    ttl: Date.now() + 5 * 60 * 1000,  // 5 分钟
  })
  
  return result
}
```

---

## 11. 附录：API 速查表

### 11.1 License API

| 方法 | 端点 | 说明 |
|------|------|------|
| `POST` | `/v1/licenses/activate` | 激活 License |
| `POST` | `/v1/licenses/validate` | 验证 License |
| `POST` | `/v1/licenses/deactivate` | 停用 License |

### 11.2 关键 Webhook 事件

| 事件名称 | 触发时机 | 关键数据 |
|---------|---------|---------|
| `checkout.completed` | 支付完成 | orderId, customerId, licenseKey |
| `license.activated` | License 被激活 | licenseId, key, deviceId |
| `license.deactivated` | License 被停用 | licenseId, key |
| `subscription.paid` | 订阅扣款成功 | subscriptionId, currentPeriodEnd |
| `subscription.renewed` | 订阅续费 | subscriptionId, currentPeriodEnd |
| `subscription.expired` | 订阅过期 | subscriptionId |
| `subscription.canceled` | 订阅取消 | subscriptionId, cancelReason |
| `subscription.past_due` | 扣款失败 | subscriptionId, attemptCount |

### 11.3 环境配置速查

| 项目 | 测试 | 生产 |
|------|------|------|
| API Base URL | `https://test-api.creem.io/v1` | `https://api.creem.io/v1` |
| API Key 格式 | `creem_test_XXXXX` | `creem_XXXXX` |
| SDK Server 参数 | `{ server: 'test' }` | 默认（或 `{ server: 'live' }`） |
| Dashboard Mode | Test Mode toggle | Live Mode |
| 支付方式 | 测试卡号 | 真实支付 |

### 11.4 测试卡号速查

| 卡号 | 用途 |
|------|------|
| `4242 4242 4242 4242` | 支付成功 |
| `4000 0000 0000 0002` | 卡被拒绝 |
| `4000 0000 0000 0069` | 过期卡 |
| `4000 0000 0000 0127` | 余额不足 |
| `4000 0000 0000 2037` | 需要 3DS 验证 |
| `4000 0000 0000 3220` | 需要 OTP 验证 |
| `4000 0000 0000 3063` | 支付争议风险 |

> **注意**：任意有效的未来日期（MM/YY）和任意 3 位 CVC 均可用于测试。

### 11.5 有用链接

| 资源 | 链接 |
|------|------|
| Creem 官网 | https://creem.io |
| 文档首页 | https://docs.creem.io |
| API Reference | https://docs.creem.io/api-reference |
| Dashboard | https://dashboard.creem.io |
| License Keys 文档 | https://docs.creem.io/features/license-keys |
| Subscriptions 文档 | https://docs.creem.io/features/subscriptions |
| Webhooks 文档 | https://docs.creem.io/code/webhooks |
| Test Mode 文档 | https://docs.creem.io/getting-started/test-mode |
| SDK (npm) | https://www.npmjs.com/package/creem |
| GitHub | https://github.com/armitage-labs/creem |

---

## 总结：完整数据流

```
┌─────────────────────────────────────────────────────────────────────────┐
│                        Creem 集成完整数据流                               │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│  【用户侧】                                                              │
│                                                                         │
│  用户访问你的 Storefront                                                  │
│       │                                                                 │
│       ▼                                                                 │
│  选择套餐（年付 $199 / 买断 $499）                                       │
│       │                                                                 │
│       ▼                                                                 │
│  Creem Checkout 页面                                                    │
│       │                                                                 │
│       ▼                                                                 │
│  输入支付信息 → 完成支付                                                 │
│       │                                                                 │
│       ▼                                                                 │
│  Creem 自动生成 License Key                                              │
│       │                                                                 │
│       ├──────────────────────────────────────────────┐                   │
│       ▼                                              ▼                   │
│  【Webhook 流】                                【用户操作流】              │
│       │                                              │                   │
│       ▼                                              ▼                   │
│  checkout.completed                           用户打开 App               │
│       │                                         │                       │
│       ▼                                         ▼                       │
│  你的后端接收事件                            输入 License Key            │
│       │                                         │                       │
│       ▼                                         ▼                       │
│  保存 License 到你的 DB                      POST /api/license/activate  │
│  (含 planType, productId...)                      │                       │
│       │                                         ▼                       │
│       ▼                                     你的后端调 Creem API         │
│  subscription.paid / renewed                   │ 激活 + 绑定设备         │
│       │                                         │                       │
│       ▼                                         ▼                       │
│  更新 updateEligible=true                  返回授权状态给 App            │
│       │                                         │                       │
│       ▼                                         ▼                       │
│  subscription.expired / canceled          App 启动时定期验证             │
│       │                                     POST /api/license/validate  │
│       ▼                                         │                       │
│  更新 updateEligible=false                 返回最新状态                 │
│       │                                         │                       │
│       ▼                                         ▼                       │
│  ┌─────────────────────────────────────────────────┐                    │
│  │              App 侧根据状态执行                   │                    │
│  │                                                │                    │
│  │  annual + active     → 全部功能 ✅ 更新 ✅       │                    │
│  │  annual + expired    → 功能 ✅ 更新 ❌ 提示续费   │                    │
│  │  lifetime + active   → 全部功能 ✅ 更新 ✅       │                    │
│  │  suspended           → 全部禁用 ❌               │                    │
│  └─────────────────────────────────────────────────┘                    │
│       │                                                                 │
│       ▼                                                                 │
│  【更新流程（如果 updateEligible=true）】                                  │
│       │                                                                 │
│       ▼                                                                 │
│  App 请求 /api/version/latest                                           │
│       │                                                                 │
│       ▼                                                                 │
│  有新版本？→ POST /api/version/download（需授权验证）                     │
│       │                                                                 │
│       ▼                                                                 │
│  返回签名下载 URL → App 下载并安装更新                                    │
│                                                                         │
└─────────────────────────────────────────────────────────────────────────┘
```

---

> **文档版本**: v1.0  
> **最后更新**: 2025-07-08  
> **基于**: Creem 官方文档 (https://docs.creem.io)  
> **适用范围**: SaaS、桌面应用、移动应用的 License 管理与订阅付费集成
