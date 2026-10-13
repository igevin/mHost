import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useNavigate } from "react-router-dom";
import { useAtomValue, useSetAtom } from "jotai";
import {
  paletteOpenAtom,
  paletteQueryAtom,
  paletteResultsAtom,
  paletteHistoryAtom,
  queryDomainAtom,
  pushPaletteHistoryAtom,
  profilesAtom,
  dnsProfilesAtom,
  dnsEnabledAtom,
  enabledProfileAtom,
  quickApplyOnToggleAtom,
  previewApplyAtom,
  quickApplyToggleAtom,
  toggleDnsProfileEnabledAtom,
  toggleDnsModeAtom,
  createProfileAtom,
  fetchProfilesAtom,
  isLoadingAtom,
} from "../stores/profiles";
import { fuzzyMatch, fuzzyRank } from "../lib/fuzzy";
import { extractErrorMessage } from "../lib/error";
import { useWebKitPointerDown } from "../hooks/useWebKitPointerDown";
import type { Profile, RuleMatch } from "../types";
import CreateProfileDialog from "./CreateProfileDialog";
import ImportDialog from "./ImportDialog";
import styles from "./CommandPalette.module.css";

/**
 * Cmd+K 命令面板（issue #124）—— 域名中心的产品命令中心。
 *
 * 两种内容：
 *  - 域名查找：非空查询经 100ms debounce 走 `query_domain` IPC（后端
 *    聚合 DNS 规则 / hosts profile / system hosts，带优先级与 WIN 标记）。
 *    Enter/点击 → 跳转对应视图并定位到行（location.state.focusLine）。
 *  - 命令：空查询时的默认列表 + 输入时的 fuzzy 过滤（手搓 subsequence
 *    matcher，见 lib/fuzzy.ts）。profile 开关走与侧栏 toggle 完全相同的
 *    preview / quick-apply 流。
 *
 * 只读纪律：面板本身绝不改规则，只导航或触发既有 action atoms。
 */

/** 单个可执行条目（扁平列表，↑↓ 在这上面走）。 */
type PaletteItem =
  | { kind: "match"; match: RuleMatch }
  | { kind: "command"; id: string; title: string; hint?: string }
  | { kind: "history"; query: string };

/** IPC 返回的匹配最多渲染这么多条 —— 大 profile 下保护首屏渲染。 */
const MAX_MATCH_ROWS = 50;

function sourceLabel(match: RuleMatch): string {
  switch (match.source.type) {
    case "system_hosts":
      return `System hosts · :${match.source.line}`;
    case "profile":
      return `${match.source.name} · :${match.source.line}`;
    case "dns_rule":
      return `DNS · ${match.source.name} · :${match.source.line}`;
  }
}

/** 按 fuzzy 位置把 domain 切成 <mark> 高亮片段。 */
function HighlightedDomain({ domain, query }: { domain: string; query: string }) {
  const match = fuzzyMatch(query, domain);
  if (!match) return <>{domain}</>;
  const marked = new Set(match.positions);
  const nodes: React.ReactNode[] = [];
  let run = "";
  let inMark = false;
  for (let i = 0; i < domain.length; i++) {
    const m = marked.has(i);
    if (m !== inMark) {
      if (run) {
        nodes.push(
          inMark ? <mark key={i}>{run}</mark> : <span key={i}>{run}</span>,
        );
      }
      run = "";
      inMark = m;
    }
    run += domain[i];
  }
  if (run) {
    nodes.push(
      inMark ? <mark key="last">{run}</mark> : <span key="last">{run}</span>,
    );
  }
  return <>{nodes}</>;
}

function CommandPalette() {
  const navigate = useNavigate();
  const open = useAtomValue(paletteOpenAtom);
  const setOpen = useSetAtom(paletteOpenAtom);
  const query = useAtomValue(paletteQueryAtom);
  const setQuery = useSetAtom(paletteQueryAtom);
  const results = useAtomValue(paletteResultsAtom);
  const history = useAtomValue(paletteHistoryAtom);
  const runQuery = useSetAtom(queryDomainAtom);
  const pushHistory = useSetAtom(pushPaletteHistoryAtom);

  const profiles = useAtomValue(profilesAtom);
  const dnsProfiles = useAtomValue(dnsProfilesAtom);
  const dnsEnabled = useAtomValue(dnsEnabledAtom);
  const enabledProfile = useAtomValue(enabledProfileAtom);
  const quickApplyEnabled = useAtomValue(quickApplyOnToggleAtom);
  const previewApply = useSetAtom(previewApplyAtom);
  const quickApplyToggle = useSetAtom(quickApplyToggleAtom);
  const toggleDnsProfile = useSetAtom(toggleDnsProfileEnabledAtom);
  const toggleDnsMode = useSetAtom(toggleDnsModeAtom);
  const createProfile = useSetAtom(createProfileAtom);
  const fetchProfiles = useSetAtom(fetchProfilesAtom);
  const isLoading = useAtomValue(isLoadingAtom);

  const [activeIndex, setActiveIndex] = useState(0);
  const [showCreate, setShowCreate] = useState(false);
  const [showImport, setShowImport] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  // 行是「输入框 → 点击目标」的焦点转移场景 —— WebKit 吞点击 quirk
  // 的兜底（pattern A：execute 幂等，面板随即关闭）。
  const { onPointerDown } = useWebKitPointerDown();

  const close = useCallback(() => {
    setOpen(false);
    setActionError(null);
  }, [setOpen]);

  // PR #244 review (F1): 行在 pointerdown 就执行（WebKit 吞点击 quirk
  // 的兜底），同步 unmount 会让同一手势的 trailing click 穿透到面板下
  // 方的页面元素（侧栏 toggle / Enable 按钮）——正是 PopupMenu 注释里
  // 记录的那类坑。执行后把关闭推迟 50ms，让 trailing click 落在还挂着
  // 的 overlay 上被吞掉。Esc / overlay 点击不是 pointerdown 执行路径，
  // 仍然同步关闭。
  // 计时器挂 ref 并在 unmount 时清掉 —— 否则面板卸载后迟到的 close()
  // 会写到共享的 paletteOpenAtom，把下一次打开又关掉。
  const closeTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const closeDeferred = useCallback(() => {
    if (closeTimerRef.current) clearTimeout(closeTimerRef.current);
    closeTimerRef.current = setTimeout(() => {
      closeTimerRef.current = null;
      close();
    }, 50);
  }, [close]);

  useEffect(() => {
    return () => {
      if (closeTimerRef.current) clearTimeout(closeTimerRef.current);
    };
  }, []);

  // 打开时清空上次查询、聚焦输入框。
  useEffect(() => {
    if (open) {
      setQuery("");
      setActiveIndex(0);
      setActionError(null);
      inputRef.current?.focus();
    }
  }, [open, setQuery]);

  // PR #244 review (F3): Esc 用 document 的 **capture** 阶段监听 +
  // stopPropagation。面板 z-index 1300 是最顶层 —— 一次 Esc 只关它，
  // 不能透传给下层 ConfirmDialog / PopupMenu 的 document 级（bubble）
  // 监听，否则一次 Esc 把两层一起关掉。
  useEffect(() => {
    if (!open) return;
    const handleEsc = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        close();
      }
    };
    document.addEventListener("keydown", handleEsc, { capture: true });
    return () => document.removeEventListener("keydown", handleEsc, { capture: true });
  }, [open, close]);

  // 100ms debounce 的域名查询 IPC（issue #124 验收：结果 ≤200ms）。
  useEffect(() => {
    if (!open) return;
    const timer = setTimeout(() => {
      runQuery(query);
    }, 100);
    return () => clearTimeout(timer);
  }, [open, query, runQuery]);

  // ---- 命令表 ----

  const toggleHostsProfile = useCallback(
    (profile: Profile) => {
      // 与 Sidebar 的 toggle 同一条路径（issue #123 语义）。
      if (quickApplyEnabled) {
        quickApplyToggle({
          id: profile.id,
          enabled: !profile.enabled,
          forcePreview: false,
        });
      } else {
        previewApply({ id: profile.id, enabled: !profile.enabled });
      }
    },
    [quickApplyEnabled, quickApplyToggle, previewApply],
  );

  const staticCommands = useMemo(() => {
    const cmds: { id: string; title: string; hint?: string }[] = [
      { id: "create-profile", title: "Create new profile" },
    ];
    if (enabledProfile) {
      cmds.push({
        id: "apply-current",
        title: `Apply current profile (${enabledProfile.name})`,
        hint: "Re-write /etc/hosts",
      });
    }
    cmds.push(
      {
        id: "toggle-dns-mode",
        title: dnsEnabled
          ? "Switch to Hosts mode (disable DNS)"
          : "Switch to DNS mode",
      },
      { id: "open-settings", title: "Open settings" },
      { id: "import-profile", title: "Import profile…", hint: "Text or file" },
    );
    return cmds;
  }, [enabledProfile, dnsEnabled]);

  const profileCommands = useMemo(() => {
    const hosts = profiles.map((p) => ({
      id: `toggle-hosts-${p.id}`,
      title: `${p.enabled ? "Disable" : "Enable"} profile "${p.name}"`,
      profile: p,
      dns: false,
    }));
    const dns = dnsProfiles.map((p) => ({
      id: `toggle-dns-${p.id}`,
      title: `${p.enabled ? "Disable" : "Enable"} DNS profile "${p.name}"`,
      profile: p,
      dns: true,
    }));
    return [...hosts, ...dns];
  }, [profiles, dnsProfiles]);

  // ---- 条目装配 ----

  const items = useMemo(() => {
    const trimmed = query.trim();
    const matches: RuleMatch[] = (
      trimmed && results && results.query === trimmed ? results.matches : []
    ).slice(0, MAX_MATCH_ROWS);

    const items: PaletteItem[] = matches.map((match) => ({ kind: "match", match }));

    if (!trimmed) {
      // 空查询：最近查询 + 默认命令列表（issue #124 的 default list）。
      for (const h of history) {
        items.push({ kind: "history", query: h });
      }
      for (const c of staticCommands) {
        items.push({ kind: "command", id: c.id, title: c.title, hint: c.hint });
      }
      return items;
    }

    const ranked = [
      ...fuzzyRank(trimmed, staticCommands, (c) => c.title),
      ...fuzzyRank(trimmed, profileCommands, (c) => c.title),
    ];
    for (const r of ranked) {
      items.push({
        kind: "command",
        id: r.item.id,
        title: r.item.title,
        hint: "hint" in r.item ? r.item.hint : undefined,
      });
    }
    return items;
  }, [query, results, history, staticCommands, profileCommands]);

  // 条目集合变化时把 active 行钳回顶部。
  useEffect(() => {
    setActiveIndex(0);
  }, [query, items.length]);

  // ↑↓ 时保持 active 行可见。
  useEffect(() => {
    listRef.current
      ?.querySelector(`[data-index="${activeIndex}"]`)
      ?.scrollIntoView?.({ block: "nearest" });
  }, [activeIndex]);

  // ---- 执行 ----

  const jumpToMatch = useCallback(
    (match: RuleMatch) => {
      pushHistory(query);
      closeDeferred();
      // jumpId nonce（review F6）：同一个 profile 同一行的重复跳转也要
      // 重新滚动 —— focusLine 是 primitive，effect dep 不变就不重跑。
      const state = { focusLine: match.source.line, jumpId: Date.now() };
      switch (match.source.type) {
        case "profile":
          navigate(`/profiles/${match.source.id}`, { state });
          break;
        case "dns_rule":
          navigate(`/dns-profiles/${match.source.id}`, { state });
          break;
        case "system_hosts":
          navigate("/hosts", { state });
          break;
      }
    },
    [pushHistory, query, closeDeferred, navigate],
  );

  const runCommand = useCallback(
    (id: string) => {
      if (id === "create-profile") {
        closeDeferred();
        setShowCreate(true);
        return;
      }
      if (id === "import-profile") {
        closeDeferred();
        setShowImport(true);
        return;
      }
      if (id === "open-settings") {
        closeDeferred();
        navigate("/settings");
        return;
      }
      if (id === "toggle-dns-mode") {
        closeDeferred();
        toggleDnsMode(!dnsEnabled);
        return;
      }
      if (id === "apply-current") {
        closeDeferred();
        if (enabledProfile) {
          previewApply({ id: enabledProfile.id, enabled: true });
        }
        return;
      }
      const cmd = profileCommands.find((c) => c.id === id);
      if (cmd) {
        closeDeferred();
        if (cmd.dns) {
          toggleDnsProfile({ id: cmd.profile.id, enabled: !cmd.profile.enabled });
        } else {
          toggleHostsProfile(cmd.profile);
        }
      }
    },
    [
      closeDeferred,
      navigate,
      toggleDnsMode,
      dnsEnabled,
      enabledProfile,
      previewApply,
      profileCommands,
      toggleDnsProfile,
      toggleHostsProfile,
    ],
  );

  const execute = useCallback(
    (item: PaletteItem) => {
      if (item.kind === "match") {
        jumpToMatch(item.match);
      } else if (item.kind === "command") {
        runCommand(item.id);
      } else {
        // 历史条目：回填查询（Enter reuses），不直接执行。
        setQuery(item.query);
        inputRef.current?.focus();
      }
    },
    [jumpToMatch, runCommand, setQuery],
  );

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      // Esc 由 document capture 监听处理（F3）—— 这里只管 ↑↓/Enter。
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setActiveIndex((i) => (items.length === 0 ? 0 : (i + 1) % items.length));
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        setActiveIndex((i) =>
          items.length === 0 ? 0 : (i - 1 + items.length) % items.length,
        );
      } else if (e.key === "Enter") {
        e.preventDefault();
        const item = items[activeIndex];
        if (item) execute(item);
      }
    },
    [items, activeIndex, execute],
  );

  const handleCreate = useCallback(
    async (name: string) => {
      try {
        const profile = await createProfile(name);
        setShowCreate(false);
        navigate(`/profiles/${profile.id}`);
      } catch (err) {
        setActionError(extractErrorMessage(err));
      }
    },
    [createProfile, navigate],
  );

  const handleImported = useCallback(
    async (profile: Profile) => {
      setShowImport(false);
      await fetchProfiles();
      navigate(`/profiles/${profile.id}`);
    },
    [fetchProfiles, navigate],
  );

  return (
    <>
      {open &&
        createPortal(
          <div
            className={styles.overlay}
            onClick={close}
            data-testid="palette-overlay"
          >
            <div
              className={styles.palette}
              role="dialog"
              aria-modal="true"
              aria-label="Command palette"
              onClick={(e) => e.stopPropagation()}
              onKeyDown={handleKeyDown}
            >
              <input
                ref={inputRef}
                className={styles.input}
                placeholder="Type a domain or command..."
                aria-label="Command palette input"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                spellCheck={false}
              />
              {actionError && (
                <div className="alert alert-error">{actionError}</div>
              )}
              <div className={styles.list} ref={listRef} role="listbox">
                {items.length === 0 && (
                  <div className={styles.empty}>
                    {query.trim() ? "No matches" : "No recent queries"}
                  </div>
                )}
                {items.map((item, index) => {
                  const active = index === activeIndex;
                  const rowClass = `${styles.row} ${active ? styles.rowActive : ""} ${
                    item.kind === "match" && !item.match.effective
                      ? styles.rowDisabled
                      : ""
                  }`;
                  return (
                    <div
                      key={
                        item.kind === "match"
                          ? `m-${item.match.source.type}-${"id" in item.match.source ? item.match.source.id : "sys"}-${item.match.domain}-${item.match.ip}`
                          : item.kind === "command"
                            ? `c-${item.id}`
                            : `h-${item.query}`
                      }
                      role="option"
                      aria-selected={active}
                      data-index={index}
                      className={rowClass}
                      onMouseEnter={() => setActiveIndex(index)}
                      onClick={() => execute(item)}
                      onPointerDown={onPointerDown(() => execute(item))}
                    >
                      {item.kind === "match" && (
                        <>
                          <span className={styles.domain}>
                            <HighlightedDomain
                              domain={item.match.domain}
                              query={query.trim()}
                            />
                            <span className={styles.ip}> → {item.match.ip}</span>
                          </span>
                          {item.match.is_winner && (
                            <span className={styles.winBadge}>WIN</span>
                          )}
                          {/* R1: resolver 后缀命中（fix #79 镜像）——
                              规则域名是查询的父域，标注出来避免误读 */}
                          {item.match.via_suffix && (
                            <span className={styles.suffixBadge}>suffix match</span>
                          )}
                          <span className={styles.sourceLabel}>
                            {sourceLabel(item.match)}
                          </span>
                        </>
                      )}
                      {item.kind === "command" && (
                        <>
                          <span className={styles.commandTitle}>{item.title}</span>
                          {item.hint && (
                            <span className={styles.sourceLabel}>{item.hint}</span>
                          )}
                        </>
                      )}
                      {item.kind === "history" && (
                        <>
                          <span className={styles.commandTitle}>{item.query}</span>
                          <span className={styles.sourceLabel}>Recent</span>
                        </>
                      )}
                    </div>
                  );
                })}
              </div>
            </div>
          </div>,
          document.body,
        )}

      {/* 面板只负责打开这两个对话框 —— 实际的创建/导入逻辑复用既有组件。 */}
      <CreateProfileDialog
        open={showCreate}
        onClose={() => setShowCreate(false)}
        onCreate={handleCreate}
        isLoading={isLoading}
      />
      <ImportDialog
        open={showImport}
        onClose={() => setShowImport(false)}
        onImported={handleImported}
      />
    </>
  );
}

export default CommandPalette;
