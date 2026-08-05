import { CloudUpload } from "lucide-react";
import type { AppSettings } from "../lib/api";
import { useI18n } from "../lib/i18n";
import { FOCUS } from "./Button";

/// Whether a server address is this computer rather than another one.
///
/// Parsed with `URL`, not split by hand: the backend validates the same string
/// with the crate `reqwest` parses with, and two parsers that disagree about
/// where the host ends is exactly the bug that let `http://evil.com\@127.0.0.1`
/// past the address check. An unparseable value is treated as leaving — a badge
/// that stays quiet because it could not read the setting is worse than one that
/// warns about a typo.
function isThisMachine(url: string): boolean {
  try {
    const host = new URL(url).hostname.toLowerCase();
    return (
      host === "localhost" ||
      host === "::1" ||
      host === "[::1]" ||
      host.startsWith("127.") ||
      // The unspecified addresses. `domain::endpoint::is_local` accepts them,
      // and a server told to bind everything is still this machine when it is
      // this machine you dial — so calling them egress would put a warning on
      // screen for a request that never leaves.
      host === "0.0.0.0" ||
      host === "::" ||
      host === "[::]"
    );
  } catch {
    return false;
  }
}

/// What this configuration sends off the machine, said out loud.
///
/// The product's whole claim is that a meeting stays here. A claim is only worth
/// something if it can be checked, and until now the only way to check was to
/// open Settings and read two dropdowns — so a user who had picked a cloud model
/// once had nothing on screen telling them their audio was leaving.
///
/// Nothing renders when nothing leaves, which is the common case and the one
/// that needs no chrome. A badge that said "private" on every screen would be
/// decoration, and decoration is exactly what stops being read.
export function EgressBadge({
  settings,
  onOpenSettings,
}: {
  settings: AppSettings;
  onOpenSettings: () => void;
}) {
  const { t } = useI18n();
  // A server the user runs is held to a loopback or private address, and half
  // of that range is another computer. `192.168.1.50` is somebody's homelab
  // box: the transcript leaves this machine to reach it.
  //
  // Offline mode does NOT stop this one — `LlmService` skips the refusal for
  // `openai_compatible` on purpose, because a server on this network is not the
  // cloud. It is still not this machine, so the badge says so. An early return
  // on `offline_mode` was hiding exactly the case the switch does not cover.
  const lan =
    settings.llm_provider === "openai_compatible" &&
    !isThisMachine(settings.endpoint_base_url);
  const cloud = !settings.offline_mode;
  const audio = cloud && settings.stt_provider === "openrouter";
  const text = lan || (cloud && settings.llm_provider === "openrouter");
  if (!audio && !text) return null;

  // Named separately rather than "cloud enabled": which of the two is leaving is
  // the whole content. Audio is the sensitive one, and a user who chose a cloud
  // summary may not realise the recording is staying put.
  const what = audio && text ? "both" : audio ? "audio" : "text";

  return (
    <button
      type="button"
      onClick={onOpenSettings}
      title={t(`egress.${what}`)}
      className={`pointer-events-auto flex h-6 items-center gap-1.5 rounded-xs border border-warn/30 bg-warn/10 px-2 text-2xs font-medium text-warn transition-colors hover:bg-warn/20 ${FOCUS}`}
    >
      <CloudUpload size={12} aria-hidden />
      {t(`egress.${what}`)}
    </button>
  );
}
