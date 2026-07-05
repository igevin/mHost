import type { ProfileMode } from "../types";
import HostsProfileView from "../components/HostsProfileView";
import DnsProfileView from "../components/DnsProfileView";

interface ProfileViewProps {
  mode?: ProfileMode;
}

/**
 * ProfileView 是 hosts / dns 模式的 thin router。
 *
 * **fix (P-F4, issue #90)**: 之前 ProfileView 同时订阅 hosts + DNS 两套
 * atoms（共 16+ 个）。hosts 模式用户编辑/查看 profile 时，DNS 侧的
 * fetchDnsProfiles / toggleDnsProfile 等都会让 ProfileView 重渲染，
 * 反之亦然。
 *
 * 拆分后：
 *   - HostsProfileView: 只订阅 profilesAtom / isLoadingAtom / errorAtom 等
 *     hosts atoms，对 DNS atom 完全不可见。
 *   - DnsProfileView: 只订阅 dnsProfilesAtom / isDnsLoadingAtom / dnsErrorAtom
 *     等 DNS atoms，对 hosts atom 完全不可见。
 *   - 本组件本身不订阅任何 atom，根据 mode 路由到对应子组件。
 *
 * JSX 渲染时按 mode 选择，route 切换会卸载一个子组件、挂载另一个，
 * 不存在双订阅问题。
 */
function ProfileView({ mode = "hosts" }: ProfileViewProps) {
  if (mode === "dns") {
    return <DnsProfileView />;
  }
  return <HostsProfileView />;
}

export default ProfileView;