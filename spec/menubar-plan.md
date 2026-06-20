# macOS 菜单栏状态栏 — 开发计划

创建日期：2026-06-20
修订日期：2026-06-20（v2：审查修订 + TDD 改造）
分支：`feature/menubar-status`

---

## 1. 阶段目标

为 mHost 添加 macOS 系统菜单栏（System Tray）支持，实现"关闭主窗口 → 隐藏到菜单栏"的典型 macOS 菜单栏应用体验。用户可以通过菜单栏图标快速切换 Profile、刷新规则、打开主窗口或退出应用。

---

## 2. 关键决策（已确认）

| 决策项 | 结论 |
|--------|------|
| 菜单类型 | 原生系统菜单（`tauri::tray::TrayIconBuilder` + `Menu`） |
| 窗口行为 | 关闭主窗口 → 隐藏到菜单栏；菜单栏 "打开主窗口" → 显示窗口 |
| 广告屏蔽 | 菜单项保留但标灰（disabled），不可交互 |
| 环境切换 | 使用 `CheckMenuItem`，checked 标识当前激活项，点击切换 |
| 快捷键 | ⌘R 刷新规则、⌘O 打开主窗口（应用前台时生效） |
| 状态同步 | Rust 命令 handler 中直接更新 tray 菜单；菜单栏切换后通过 `app.emit()` 通知前端 |
| 图标 | 使用 template image 适配 macOS 深色/浅色菜单栏 |
| 开发方式 | TDD（Red-Green-Refactor），可测试逻辑与系统 UI 交互分离 |

---

## 3. 原生菜单 vs 原型的差异

由于使用原生系统菜单，以下原型元素会简化：

| 原型元素 | 原生菜单实现 |
|----------|-------------|
| 彩色环境圆点（蓝/绿/橙/红） | `CheckMenuItem` checked 状态标识当前激活项 |
| Toggle 开关（广告屏蔽） | 标准菜单项，标灰 disabled |
| 统计卡片面板（紧凑菜单变体） | 不适用，仅保留完整菜单变体 |
| 状态指示 pip（在线/离线） | 通过 tooltip 文本体现（如 "mHost - Development 已启用"） |

---

## 4. 菜单结构设计

```
┌─────────────────────────────┐
│  mHost - Development 已启用  │  ← tooltip（动态更新）
├─────────────────────────────┤
│  ▸ 环境配置                   │  ← Submenu
│      ✓ Development           │  ← CheckMenuItem (checked)
│        Testing               │  ← CheckMenuItem (unchecked)
│        Staging               │  ← CheckMenuItem (unchecked)
│        Production            │  ← CheckMenuItem (unchecked)
├─────────────────────────────┤
│  广告屏蔽（即将推出）          │  ← MenuItem (disabled, greyed out)
├─────────────────────────────┤
│  刷新远程规则        ⌘R      │  ← MenuItem
│  打开主窗口          ⌘O      │  ← MenuItem
├─────────────────────────────┤
│  v0.1.0-alpha       退出      │  ← Separator + Quit
└─────────────────────────────┘
```

**MenuId 命名规范**：

| 菜单项 | MenuId |
|--------|--------|
| 环境配置 Submenu | `"profiles_submenu"` |
| Profile 项 | `"profile.{profile_id}"` |
| 广告屏蔽 | `"adblock"` |
| 刷新远程规则 | `"refresh_rules"` |
| 打开主窗口 | `"open_window"` |
| 退出 | `"quit"` |

---

## 5. 技术实现方案

### 5.1 架构分层：可测试逻辑与系统 UI 交互分离

TDD 的核心挑战是 tray 菜单涉及系统 UI（无法在 CI 中单元测试）。解决方案是将逻辑分为两层：

```
┌──────────────────────────────────────┐
│  tray.rs（系统 UI 层，非 TDD）         │
│  - build_tray()：构建 TrayIcon + Menu │
│  - handle_menu_event()：事件分发       │
│  - update_tray_menu()：调用下层更新    │
└──────────────┬───────────────────────┘
               │ 调用
┌──────────────▼───────────────────────┐
│  tray_logic.rs（纯逻辑层，TDD）       │
│  - build_menu_state()：计算菜单状态   │
│  - resolve_menu_action()：解析菜单动作 │
│  - build_tooltip_text()：生成 tooltip  │
│  - emit_frontend_event()：前端通知决策 │
└──────────────────────────────────────┘
```

**`tray_logic.rs`** 是纯函数模块，不依赖 Tauri API，所有输入输出都是普通 Rust 类型，可以完整 TDD。

**`tray.rs`** 是薄封装层，负责将 `tray_logic.rs` 的计算结果转换为 Tauri API 调用。

### 5.2 Rust 端：纯逻辑层（TDD）

```rust
// src-tauri/src/tray_logic.rs

/// 菜单项状态（纯数据，不依赖 Tauri API）
#[derive(Debug, Clone, PartialEq)]
pub enum TrayMenuAction {
    SwitchProfile(String),    // profile_id (UUID string)
    RefreshRules,
    OpenWindow,
    Quit,
    AdBlock,                 // disabled，不应被触发
    Unknown,
}

/// 从 MenuEvent 的 id 字符串解析出动作
pub fn resolve_menu_action(menu_id: &str) -> TrayMenuAction {
    match menu_id {
        id if id.starts_with("profile.") => {
            TrayMenuAction::SwitchProfile(id.strip_prefix("profile.").unwrap().to_string())
        }
        "refresh_rules" => TrayMenuAction::RefreshRules,
        "open_window" => TrayMenuAction::OpenWindow,
        "quit" => TrayMenuAction::Quit,
        "adblock" => TrayMenuAction::AdBlock,
        _ => TrayMenuAction::Unknown,
    }
}

/// Profile 菜单项的显示状态
#[derive(Debug, Clone, PartialEq)]
pub struct ProfileMenuItem {
    pub profile_id: String,
    pub name: String,
    pub checked: bool,
}

/// 计算当前菜单应显示的 Profile 列表
pub fn build_profile_menu_items(
    profiles: &[(String, String, bool)],  // (id, name, enabled)
) -> Vec<ProfileMenuItem> {
    profiles.iter().map(|(id, name, enabled)| {
        ProfileMenuItem {
            profile_id: id.clone(),
            name: name.clone(),
            checked: *enabled,
        }
    }).collect()
}

/// 生成 tooltip 文本
pub fn build_tooltip_text(enabled_profile_name: Option<&str>) -> String {
    match enabled_profile_name {
        Some(name) => format!("mHost - {} 已启用", name),
        None => "mHost - 未启用".to_string(),
    }
}

/// 判断是否需要重建菜单（Profile 列表变化时需要重建，仅 checkmark 变化时不需要）
#[derive(Debug, Clone, PartialEq)]
pub enum MenuUpdateKind {
    CheckOnly,       // 仅更新 checkmark，不需要重建菜单
    Rebuild,         // Profile 列表变化，需要重建菜单
}

pub fn determine_menu_update_kind(
    old_profile_ids: &[String],
    new_profile_ids: &[String],
) -> MenuUpdateKind {
    if old_profile_ids == new_profile_ids {
        MenuUpdateKind::CheckOnly
    } else {
        MenuUpdateKind::Rebuild
    }
}
```

### 5.3 Rust 端：系统 UI 层（非 TDD）

```rust
// src-tauri/src/tray.rs

use tauri::{
    AppHandle, Manager,
    menu::{Menu, MenuItem, Submenu, PredefinedMenuItem, CheckMenuItem},
    tray::TrayIconBuilder,
    image::Image,
};
use crate::tray_logic::*;
use crate::state::AppState;

const TRAY_ICON_ID: &str = "main-tray";

/// 构建初始 tray 菜单（启动时调用一次）
pub fn build_tray(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let state = app.state::<AppState>();
    let profiles = state.storage.list_profiles()?;
    let profile_data: Vec<_> = profiles.iter()
        .map(|p| (p.id.0.to_string(), p.name.clone(), p.enabled))
        .collect();

    let items = build_profile_menu_items(&profile_data);
    let enabled_name = profiles.iter().find(|p| p.enabled).map(|p| p.name.as_str());
    let tooltip = build_tooltip_text(enabled_name);

    let profiles_submenu = build_profiles_submenu(app, &items)?;
    let menu = build_full_menu(app, &profiles_submenu)?;

    let icon = Image::from_bytes(include_bytes!("../icons/tray-icon.png"))?;

    TrayIconBuilder::with_id(TRAY_ICON_ID)
        .icon(icon)
        .icon_as_template(true)
        .menu(&menu)
        .tooltip(&tooltip)
        .show_menu_on_left_click(true)
        .on_menu_event(handle_menu_event)
        .build(app)?;

    Ok(())
}

/// 菜单事件处理
fn handle_menu_event(app: &AppHandle, event: tauri::menu::MenuEvent) {
    let action = resolve_menu_action(event.id().as_ref());
    match action {
        TrayMenuAction::SwitchProfile(profile_id) => {
            // 使用 tauri::async_runtime::spawn_blocking 避免阻塞
            // enable_and_apply_logic 包含 sudo 弹窗，需在主线程
            let app = app.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let state = app.state::<AppState>();
                if let Err(e) = do_switch_profile(&state, &profile_id) {
                    eprintln!("[mHost] Tray: switch profile failed: {}", e);
                    return;
                }
                // 更新菜单 checkmark
                update_tray_checkmark(&app);
                // 通知前端刷新
                let _ = app.emit("tray:profiles-updated", ());
            });
        }
        TrayMenuAction::OpenWindow => {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }
        TrayMenuAction::Quit => {
            app.exit(0);
        }
        _ => {}
    }
}

/// 仅更新 checkmark 状态（不重建菜单，避免闪烁）
fn update_tray_checkmark(app: &AppHandle) {
    let state = app.state::<AppState>();
    let profiles = match state.storage.list_profiles() {
        Ok(p) => p,
        Err(_) => return,
    };

    if let Some(tray) = app.tray_handle_by_id(TRAY_ICON_ID) {
        if let Ok(menu) = tray.menu() {
            if let Some(submenu) = menu.get("profiles_submenu") {
                if let tauri::menu::MenuItemKind::Submenu(sub) = submenu {
                    for item in sub.items().unwrap_or_default() {
                        if let tauri::menu::MenuItemKind::CheckMenuItem(cm) = item {
                            let id = cm.id().as_ref();
                            if let Some(pid) = id.strip_prefix("profile.") {
                                let is_enabled = profiles.iter()
                                    .any(|p| p.id.0.to_string() == pid && p.enabled);
                                let _ = cm.set_checked(is_enabled);
                            }
                        }
                    }
                }
            }
        }

        // 更新 tooltip
        let enabled_name = profiles.iter().find(|p| p.enabled).map(|p| p.name.as_str());
        let _ = tray.set_tooltip(Some(&build_tooltip_text(enabled_name)));
    }
}

/// 完整重建菜单（Profile 列表变化时调用）
pub fn update_tray_menu(app: &AppHandle) {
    // 读取当前菜单中的 profile id 列表
    let old_ids = get_current_profile_ids(app);

    let state = app.state::<AppState>();
    let profiles = match state.storage.list_profiles() {
        Ok(p) => p,
        Err(_) => return,
    };
    let new_ids: Vec<String> = profiles.iter().map(|p| p.id.0.to_string()).collect();

    let kind = determine_menu_update_kind(&old_ids, &new_ids);
    match kind {
        MenuUpdateKind::CheckOnly => update_tray_checkmark(app),
        MenuUpdateKind::Rebuild => {
            // 重建整个菜单
            let profile_data: Vec<_> = profiles.iter()
                .map(|p| (p.id.0.to_string(), p.name.clone(), p.enabled))
                .collect();
            let items = build_profile_menu_items(&profile_data);
            if let Some(tray) = app.tray_handle_by_id(TRAY_ICON_ID) {
                let submenu = match build_profiles_submenu(app, &items) {
                    Ok(s) => s,
                    Err(_) => return,
                };
                let menu = match build_full_menu(app, &submenu) {
                    Ok(m) => m,
                    Err(_) => return,
                };
                let _ = tray.set_menu(Some(menu));

                let enabled_name = profiles.iter().find(|p| p.enabled).map(|p| p.name.as_str());
                let _ = tray.set_tooltip(Some(&build_tooltip_text(enabled_name)));
            }
        }
    }
}
```

### 5.4 Rust 端：窗口关闭拦截

```rust
// src-tauri/src/lib.rs

pub fn run() {
    let app_state = match AppState::new() {
        Ok(state) => state,
        Err(e) => {
            eprintln!("[mHost] Failed to initialize AppState: {}", e);
            std::process::exit(1);
        }
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(app_state)
        .setup(|app| {
            // 构建系统菜单栏（失败仅警告，不阻止启动）
            if let Err(e) = crate::tray::build_tray(&app.handle()) {
                eprintln!("[mHost] Warning: failed to build system tray: {}", e);
            }

            // 拦截窗口关闭 → 隐藏到菜单栏
            if let Some(window) = app.get_webview_window("main") {
                let win = window.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = win.hide();
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // ... 已有 commands
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

### 5.5 前端变更

**`src/App.tsx`**：监听 `tray:profiles-updated` 事件，刷新 profile 列表。

```typescript
// src/App.tsx
import { useEffect } from 'react';
import { listen } from '@tauri-apps/api/event';

function App() {
  // ... 已有逻辑

  // 监听菜单栏 Profile 切换事件
  useEffect(() => {
    const unlisten = listen('tray:profiles-updated', () => {
      // 触发 fetchProfiles 刷新列表
      // 已有 store 中的 fetchProfilesAtom 可直接使用
    });
    return () => { unlisten.then(fn => fn()); };
  }, []);

  // ...
}
```

### 5.6 配置层变更

**`tauri.conf.json`**：显式添加窗口 label。

```json
"windows": [
  {
    "label": "main",
    "title": "mHost",
    "width": 900,
    "height": 700
  }
]
```

**`capabilities/default.json`**：无需额外权限。

---

## 6. TDD 执行原则（继承 Phase 0/1）

- **Red**：先写测试，确认失败
- **Green**：实现最小代码，让测试通过
- **Refactor**：重构，保持测试通过
- 所有 Rust 单元测试采用表格驱动
- 测试断言必须验证具体内容，不能仅验证数量

---

## 7. 任务拆分

### 总览

```
              ┌──────────────────────────────────┐
              │  T0：工程准备（feature + 依赖）    │
              └──────────────┬───────────────────┘
                             │
          ┌──────────────────┼──────────────────┐
          │                  │                  │
          ▼                  ▼                  ▼
┌──────────────────┐ ┌────────────────┐ ┌──────────────────┐
│ T1：tray_logic   │ │ T2：tray.rs    │ │ T3：窗口关闭拦截  │
│ 纯逻辑层（TDD）   │ │ 系统UI层（非TDD）│ │ + 前端事件监听    │
│                  │ │                │ │                  │
│ resolve_menu_   │ │ build_tray()   │ │ on_window_event  │
│ action()        │ │ handle_menu_   │ │ App.tsx listen   │
│ build_profile_  │ │ event()        │ │ tray:profiles-   │
│ menu_items()    │ │ update_tray_   │ │ updated          │
│ build_tooltip_  │ │ menu()         │ │                  │
│ text()          │ │ update_tray_   │ │                  │
│ determine_menu_ │ │ checkmark()    │ │                  │
│ update_kind()   │ │                │ │                  │
└────────┬─────────┘ └───────┬────────┘ └────────┬─────────┘
         │                  │                   │
         └──────────┬───────┘                   │
                    │                          │
                    └──────────┬───────────────┘
                               │
                    ┌──────────▼──────────┐
                    │ T4：图标资源          │
                    │ macOS template image │
                    └──────────┬──────────┘
                               │
                    ┌──────────▼──────────┐
                    │ T5：集成验收          │
                    └─────────────────────┘
```

---

### T0：工程准备

**类型**：基础设施（非 TDD）

**依赖**：无

**交付物**：分支创建、依赖配置、配置文件修改

**具体步骤**：

1. 创建分支 `feature/menubar-status`（已完成）
2. `src-tauri/Cargo.toml` 添加 `tray-icon` feature：
   ```toml
   tauri = { version = "2", features = ["tray-icon"] }
   ```
3. `src-tauri/tauri.conf.json` 添加窗口 label：
   ```json
   "windows": [{ "label": "main", "title": "mHost", "width": 900, "height": 700 }]
   ```
4. 验证 `cargo build` 通过（`tray-icon` feature 在 macOS 上需要 `cocoa` 框架，Xcode 环境应已具备）

**验收**：`cargo build` 通过

**预估**：0.25 天

---

### T1：tray_logic 纯逻辑层（TDD）

**类型**：TDD

**依赖**：T0

**交付物**：`src-tauri/src/tray_logic.rs` + 测试文件 `src-tauri/src/tray_logic.rs`（内联 `#[cfg(test)]` 模块）

**内容**：

实现第 5.2 节定义的所有纯函数。

**TDD 测试用例（表格驱动）**：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // ===== resolve_menu_action =====
    #[test]
    fn test_resolve_menu_action() {
        let cases = vec![
            ("profile.550e8400-e29b-41d4-a716-446655440000",
             TrayMenuAction::SwitchProfile("550e8400-e29b-41d4-a716-446655440000".into())),
            ("refresh_rules", TrayMenuAction::RefreshRules),
            ("open_window", TrayMenuAction::OpenWindow),
            ("quit", TrayMenuAction::Quit),
            ("adblock", TrayMenuAction::AdBlock),
            ("unknown_id", TrayMenuAction::Unknown),
            ("", TrayMenuAction::Unknown),
            ("profile.", TrayMenuAction::SwitchProfile("".into())),  // edge case
        ];
        for (input, expected) in cases {
            let result = resolve_menu_action(input);
            assert_eq!(result, expected, "input: {}", input);
        }
    }

    // ===== build_profile_menu_items =====
    #[test]
    fn test_build_profile_menu_items() {
        let cases = vec![
            ("empty", vec![], vec![]),
            ("single_disabled", vec![
                ("id-1".into(), "Development".into(), false),
            ], vec![
                ProfileMenuItem { profile_id: "id-1".into(), name: "Development".into(), checked: false },
            ]),
            ("single_enabled", vec![
                ("id-2".into(), "Testing".into(), true),
            ], vec![
                ProfileMenuItem { profile_id: "id-2".into(), name: "Testing".into(), checked: true },
            ]),
            ("multiple_one_enabled", vec![
                ("id-1".into(), "Dev".into(), true),
                ("id-2".into(), "Test".into(), false),
                ("id-3".into(), "Prod".into(), false),
            ], vec![
                ProfileMenuItem { profile_id: "id-1".into(), name: "Dev".into(), checked: true },
                ProfileMenuItem { profile_id: "id-2".into(), name: "Test".into(), checked: false },
                ProfileMenuItem { profile_id: "id-3".into(), name: "Prod".into(), checked: false },
            ]),
        ];
        for (name, input, expected) in cases {
            let result = build_profile_menu_items(&input);
            assert_eq!(result, expected, "case: {}", name);
        }
    }

    // ===== build_tooltip_text =====
    #[test]
    fn test_build_tooltip_text() {
        let cases = vec![
            ("with_name", Some("Development"), "mHost - Development 已启用"),
            ("none", None, "mHost - 未启用"),
            ("empty_name", Some(""), "mHost -  已启用"),
        ];
        for (name, input, expected) in cases {
            let result = build_tooltip_text(input);
            assert_eq!(result, expected, "case: {}", name);
        }
    }

    // ===== determine_menu_update_kind =====
    #[test]
    fn test_determine_menu_update_kind() {
        let cases = vec![
            ("same_order", vec!["a","b","c"], vec!["a","b","c"], MenuUpdateKind::CheckOnly),
            ("reorder", vec!["a","b","c"], vec!["c","b","a"], MenuUpdateKind::Rebuild),
            ("added", vec!["a","b"], vec!["a","b","c"], MenuUpdateKind::Rebuild),
            ("removed", vec!["a","b","c"], vec!["a","b"], MenuUpdateKind::Rebuild),
            ("empty_both", vec![], vec![], MenuUpdateKind::CheckOnly),
            ("empty_to_one", vec![], vec!["a"], MenuUpdateKind::Rebuild),
            ("one_to_empty", vec!["a"], vec![], MenuUpdateKind::Rebuild),
        ];
        for (name, old, new, expected) in cases {
            let result = determine_menu_update_kind(
                &old.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                &new.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            );
            assert_eq!(result, expected, "case: {}", name);
        }
    }
}
```

**验收**：`cargo test -p mhost` 通过（新增测试全部 green）

**预估**：0.5 天

---

### T2：tray.rs 系统UI层（非 TDD）

**类型**：后端开发（非 TDD，依赖系统 UI）

**依赖**：T1

**交付物**：`src-tauri/src/tray.rs` + `src-tauri/src/lib.rs` 修改

**具体步骤**：

1. 创建 `src-tauri/src/tray.rs`，实现：
   - `build_tray(app)` — 构建初始 TrayIcon + Menu，使用 `CheckMenuItem` 构建 Profile 项
   - `build_profiles_submenu(app, items)` — 使用 `CheckMenuItem::with_id` 构建
   - `build_full_menu(app, submenu)` — 组装完整菜单
   - `handle_menu_event(app, event)` — 事件分发，调用 `tray_logic::resolve_menu_action`
   - `update_tray_checkmark(app)` — 仅更新 checkmark（遍历 submenu 中的 CheckMenuItem 调用 `set_checked`）
   - `update_tray_menu(app)` — 判断 `MenuUpdateKind` 后选择 `CheckOnly` 或 `Rebuild`
   - `get_current_profile_ids(app)` — 从当前菜单中读取 profile id 列表
2. 修改 `src-tauri/src/lib.rs`：
   - 添加 `mod tray;` 和 `mod tray_logic;`
   - 在 `setup` 中调用 `build_tray`（失败仅警告）
3. 在以下 command handler 末尾添加 `update_tray_menu(&app_handle)` 调用：
   - `set_profile_enabled`（`commands/profile.rs`）
   - `enable_and_apply`（`commands/apply.rs`）
   - `create_profile`（`commands/profile.rs`）
   - `delete_profile`（`commands/profile.rs`）
   - `update_profile`（`commands/profile.rs`）
4. Profile 切换逻辑：
   - 不调用 Tauri command（command 需要 `State<'_, AppState>` 参数）
   - 直接调用 `enable_and_apply_logic`（纯逻辑函数）
   - 使用 `tauri::async_runtime::spawn_blocking` 避免阻塞菜单事件线程
   - 成功后调用 `update_tray_checkmark` + `app.emit("tray:profiles-updated", ())`

**关键实现细节**：

- **CheckMenuItem vs MenuItem**：Profile 项使用 `CheckMenuItem::with_id(app, id, text, enabled, checked, accelerator)`，比 `MenuItem` 多一个 `checked` 参数。运行时通过 `cm.set_checked(bool)` 更新勾选状态，无需重建菜单。
- **线程模型**：`on_menu_event` 回调在系统线程执行。`enable_and_apply_logic` 包含 sudo 弹窗（需主线程），使用 `spawn_blocking` 处理。
- **前端通知**：Profile 切换成功后通过 `app.emit("tray:profiles-updated", ())` 通知前端。

**验收**：
- `cargo build` 通过
- 应用启动后菜单栏出现 mHost 图标
- 点击图标弹出菜单，Profile 项为 CheckMenuItem
- 点击 "打开主窗口" → 窗口显示
- 点击 "退出" → 应用退出

**预估**：1 天

---

### T3：窗口关闭拦截 + 前端事件监听

**类型**：TDD（前端事件监听部分）+ 后端（非 TDD，窗口事件拦截）

**依赖**：T2

**交付物**：`src-tauri/src/lib.rs` 窗口关闭拦截 + `src/App.tsx` 事件监听

**具体步骤**：

#### T3.1 Rust 端：窗口关闭拦截

在 `lib.rs` 的 `setup` 中添加 `on_window_event` 拦截（代码见 5.4 节）。

#### T3.2 前端：监听菜单栏事件

在 `src/App.tsx` 中添加 `tray:profiles-updated` 事件监听，触发 profile 列表刷新。

**前端 TDD 测试用例**：

```typescript
describe('tray event integration', () => {
  it('listens to tray:profiles-updated event and triggers profile refresh', async () => {
    // mock listen 和 fetchProfiles
    // 验证事件触发后 fetchProfiles 被调用
  });

  it('unsubscribes from event on unmount', async () => {
    // 验证组件卸载时取消监听
  });
});
```

**验收**：
- 关闭主窗口 → 窗口隐藏，菜单栏图标仍在
- 菜单栏切换 Profile → 主窗口中 Profile 列表自动刷新
- 主窗口切换 Profile → 菜单栏 checkmark 自动更新

**预估**：0.5 天

---

### T4：图标资源

**类型**：设计（非 TDD）

**依赖**：T2

**交付物**：macOS template image 图标文件

**具体步骤**：

1. 制作 macOS template image：
   - 要求：纯黑色（`#000000`）轮廓，透明背景，无填充
   - 尺寸：22x22 @1x，44x44 @2x（macOS 推荐）
   - 放置路径：`src-tauri/icons/tray-icon.png` 和 `src-tauri/icons/tray-icon@2x.png`
2. 修改 `tray.rs` 中 `include_bytes!` 路径

**验收**：
- 浅色菜单栏下图标清晰可见
- 深色菜单栏下图标自动反色显示

**预估**：0.25 天

---

### T5：集成验收

**类型**：手动测试

**依赖**：T1-T4

**验收流程**：

1. 应用启动 → 菜单栏出现 mHost 图标
2. 点击图标 → 弹出菜单，Profile 项为 CheckMenuItem
3. 菜单中切换 Profile → hosts 文件更新 → 主窗口自动刷新
4. 主窗口中切换 Profile → 菜单 checkmark 自动更新
5. 关闭主窗口 → 窗口隐藏，图标仍在
6. 菜单 "打开主窗口" → 窗口重新显示并获得焦点
7. 菜单 "退出" → 应用退出
8. 新建 Profile → 菜单新增 CheckMenuItem
9. 删除 Profile → 菜单移除项
10. 重命名 Profile → 菜单文本更新
11. 广告屏蔽菜单项灰色不可点击
12. tooltip 显示当前激活 Profile 名称
13. ⌘R / ⌘O 快捷键在应用前台时生效
14. 深色/浅色菜单栏下图标正确显示
15. `cargo test` 全部通过
16. `pnpm vitest run` 全部通过

**预估**：0.5 天

---

## 8. 任务依赖图

```
T0（工程准备）
├── T1（tray_logic 纯逻辑层 TDD）
│   └── T2（tray.rs 系统UI层）
│       ├── T3（窗口关闭拦截 + 前端事件监听）
│       └── T4（图标资源）
│           └── T5（集成验收）
```

T3 和 T4 可在 T2 完成后并行开发。

---

## 9. 文件变更清单

| 文件 | 操作 | 说明 |
|------|------|------|
| `src-tauri/Cargo.toml` | 修改 | tauri 依赖添加 `tray-icon` feature |
| `src-tauri/src/tray_logic.rs` | **新增** | 纯逻辑层（TDD），菜单状态计算 |
| `src-tauri/src/tray.rs` | **新增** | 系统UI层，TrayIcon 构建 + 事件处理 |
| `src-tauri/src/lib.rs` | 修改 | 添加 mod 声明、setup 中构建 tray、窗口关闭拦截 |
| `src-tauri/src/commands/profile.rs` | 修改 | command handler 末尾调用 `update_tray_menu` |
| `src-tauri/src/commands/apply.rs` | 修改 | `enable_and_apply` 末尾调用 `update_tray_menu` |
| `src-tauri/tauri.conf.json` | 修改 | 窗口添加 `"label": "main"` |
| `src/App.tsx` | 修改 | 监听 `tray:profiles-updated` 事件 |
| `src-tauri/icons/tray-icon.png` | **新增** | macOS template image @1x |
| `src-tauri/icons/tray-icon@2x.png` | **新增** | macOS template image @2x |

**不需要修改的文件**：
- `capabilities/default.json` — tray 不需要额外权限

---

## 10. 阶段产出物

| 产出物 | 说明 |
|--------|------|
| `tray_logic.rs` 模块 | 纯逻辑层，菜单状态计算（TDD 覆盖） |
| `tray.rs` 模块 | 系统UI层，TrayIcon 构建 + 事件处理 |
| 窗口关闭拦截 | 关闭 → 隐藏到菜单栏 |
| Profile 菜单联动 | 前端/菜单栏双向同步（CheckMenuItem + emit） |
| 前端事件监听 | `tray:profiles-updated` 事件处理 |
| macOS template image | 适配深色/浅色菜单栏的图标 |
| 单元测试 | `tray_logic.rs` 全部纯函数的表格驱动测试 |

---

## 11. 阶段不做的事

| 排除项 | 原因 |
|--------|------|
| 自定义 WebView 下拉菜单 | 复杂度过高，原生菜单已满足需求 |
| 广告屏蔽功能 | 阶段 3 范围，菜单项仅占位 |
| 统计面板（紧凑菜单变体） | 原生菜单无法实现 |
| 在线/离线状态图标切换 | 当前阶段无此需求，预留 |
| 全局快捷键（后台响应） | 需要额外插件，作为可选增强 |
| Windows/Linux tray 适配 | macOS 优先，后续跨平台扩展 |
| Dock 菜单自定义 | 非核心需求 |

---

## 12. 风险与应对

| 优先级 | 风险 | 影响 | 应对 |
|--------|------|------|------|
| **高** | macOS template image 图标不满足要求（当前 32x32.png 是彩色图标） | 中 | T4 中制作纯黑色轮廓 template image |
| **中** | `enable_and_apply_logic` 包含 sudo 弹窗，在菜单事件线程中调用可能有问题 | 中 | 使用 `spawn_blocking`；必要时用 `app.run_on_main_thread()` |
| **中** | `TrayIcon::set_menu`/`set_tooltip` 的线程模型不确定 | 低 | T2 中实测验证；如有问题用 `run_on_main_thread` 包装 |
| **低** | 全局快捷键仅在应用前台生效 | 低 | 可接受，后续按需引入 `tauri-plugin-global-shortcut` |
| **低** | `tray-icon` feature 在 CI Linux 环境可能需要额外依赖 | 低 | CI 中添加 `libappindicator` 安装步骤 |

---

## 13. 预估总工期

| 任务 | 预估 |
|------|------|
| T0：工程准备 | 0.25 天 |
| T1：tray_logic 纯逻辑层（TDD） | 0.5 天 |
| T2：tray.rs 系统UI层 | 1 天 |
| T3：窗口关闭拦截 + 前端事件 | 0.5 天 |
| T4：图标资源 | 0.25 天 |
| T5：集成验收 | 0.5 天 |
| **合计** | **约 3 天** |

**并行后实际工期**：T0(0.25) + T1(0.5) + T2(1) + max(T3,T4)(0.5) + T5(0.5) = **2.75 个工作日**。

---

## 14. 验收标准

- [ ] `cargo test` 全部通过（含 `tray_logic.rs` 新增测试）
- [ ] `pnpm vitest run` 全部通过（含前端事件监听测试）
- [ ] `cargo build` 通过
- [ ] 应用启动后 macOS 菜单栏出现 mHost 图标
- [ ] 点击图标弹出菜单，Profile 项为 CheckMenuItem
- [ ] 通过菜单栏切换 Profile → hosts 文件更新 → 主窗口自动刷新
- [ ] 通过主窗口切换 Profile → 菜单栏 checkmark 同步更新
- [ ] 关闭主窗口 → 隐藏到菜单栏（不退出）
- [ ] 菜单 "打开主窗口" → 窗口重新显示并获得焦点
- [ ] 菜单 "退出" → 应用退出
- [ ] 新建/删除/重命名 Profile → 菜单栏菜单同步更新
- [ ] 广告屏蔽菜单项标灰不可交互
- [ ] tooltip 显示当前激活的 Profile 名称
- [ ] ⌘R / ⌘O 快捷键在应用前台时生效
- [ ] 深色/浅色菜单栏下图标正确显示（template image）

---

## 附录 A：审查修订记录

本附录记录了 v1 → v2 的修订内容，供开发者参考。

### A.1 使用 CheckMenuItem 替代 MenuItem（阻塞级）

**问题**：v1 中 Profile 项使用 `MenuItem`，需要重建整个菜单来更新 checkmark，可能导致视觉闪烁。

**决策**：改用 `CheckMenuItem::with_id(app, id, text, enabled, checked, accelerator)`。优势：
- 构建时通过 `checked` 参数设置初始状态
- 运行时通过 `cm.set_checked(bool)` 单独更新，**无需重建菜单**
- 消除了菜单闪烁风险

### A.2 前端事件通知机制（阻塞级）

**问题**：v1 中菜单栏切换 Profile 后，主窗口 UI 不会自动更新。

**决策**：在 tray 事件处理器中，Profile 切换成功后通过 `app.emit("tray:profiles-updated", ())` 向前端发送事件。前端 `App.tsx` 监听此事件并触发 profile 列表刷新。

### A.3 直接调用 enable_and_apply_logic（阻塞级）

**问题**：v1 中未明确如何从 tray 事件处理器调用已有逻辑。Tauri command 需要 `State<'_, AppState>` 参数，但 `on_menu_event` 回调签名固定为 `Fn(&AppHandle, MenuEvent)`。

**决策**：不调用 Tauri command，直接调用 `enable_and_apply_logic` 纯逻辑函数，通过 `app.state::<AppState>()` 手动获取状态。

### A.4 线程模型（重要级）

**问题**：`on_menu_event` 回调在系统线程执行，`enable_and_apply_logic` 包含 sudo 弹窗（需主线程）。

**决策**：使用 `tauri::async_runtime::spawn_blocking` 处理耗时操作。如 sudo 弹窗需要在主线程，使用 `app.run_on_main_thread()`。

### A.5 窗口 label 显式声明（建议级）

**问题**：`tauri.conf.json` 中窗口未设置 `label`，依赖 Tauri 隐式分配 `"main"`。

**决策**：显式添加 `"label": "main"`，避免依赖隐式行为。

### A.6 tray 构建失败不阻止启动（建议级）

**问题**：v1 中 `build_tray` 失败会导致应用无法启动。

**决策**：`build_tray` 失败时仅打印警告（`eprintln!`），不阻止应用启动。在非 macOS 平台上 tray 功能自动降级。

### A.7 架构分层：tray_logic.rs 纯逻辑层（TDD 核心）

**问题**：v1 中所有逻辑都在 `tray.rs` 中，无法 TDD。

**决策**：拆分为 `tray_logic.rs`（纯函数，TDD）和 `tray.rs`（系统 UI 薄封装，非 TDD）。`tray_logic.rs` 包含：
- `resolve_menu_action()` — MenuId → 动作解析
- `build_profile_menu_items()` — 计算菜单项状态
- `build_tooltip_text()` — 生成 tooltip 文本
- `determine_menu_update_kind()` — 判断是否需要重建菜单
