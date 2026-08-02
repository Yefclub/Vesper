import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Minus, Square, X, Copy as Restore } from "lucide-react";
import { useI18n } from "../lib/i18n";

/**
 * Minimise / maximise / close, drawn by the app.
 *
 * `decorations: false` in `tauri.conf.json` is what makes this necessary: the
 * system frame drew a light grey bar above a dark cream window and there was no
 * way to theme it. Owning the buttons is the only way the strip belongs to the
 * app rather than to Windows.
 *
 * Shapes and order are Windows', not invented: 46×32 targets, minimise, then
 * maximise/restore, then close, and close is the only one that turns red. A
 * custom titlebar that reorders these is the fastest way to make an app feel
 * wrong on the platform it runs on.
 */
export function WindowControls() {
  const { t } = useI18n();
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    const win = getCurrentWindow();
    let unlisten: (() => void) | undefined;
    let live = true;
    // The state has to be read once and then followed: the window can be
    // maximised by a double-click on the drag region, by the OS snap gesture,
    // or by Win+Up, none of which pass through the button below.
    void win.isMaximized().then((v) => live && setMaximized(v));
    void win
      .onResized(() => {
        void win.isMaximized().then((v) => live && setMaximized(v));
      })
      .then((f) => {
        if (live) unlisten = f;
        else f();
      });
    return () => {
      live = false;
      unlisten?.();
    };
  }, []);

  const win = getCurrentWindow();
  return (
    // No opt-out attribute here. Tauri tests the element the press landed on for
    // `data-tauri-drag-region`, and these buttons do not carry it, so they are
    // already excluded from the header's drag region. Writing `={false}` would
    // render `data-tauri-drag-region="false"` — an attribute that is *present* —
    // and turn Close into a drag handle.
    <div className="flex items-stretch">
      <ControlButton
        label={t("window.minimize")}
        onClick={() => void win.minimize()}
      >
        <Minus size={14} />
      </ControlButton>
      <ControlButton
        label={t(maximized ? "window.restore" : "window.maximize")}
        onClick={() => void win.toggleMaximize()}
      >
        {maximized ? (
          // Two offset squares — the platform's restore glyph. `scale-x-[-1]`
          // because lucide's Copy has its filled square at the front and
          // Windows draws the overlapped one behind.
          <Restore size={12} className="scale-x-[-1]" />
        ) : (
          <Square size={12} />
        )}
      </ControlButton>
      <ControlButton
        label={t("window.close")}
        onClick={() => void win.close()}
        danger
      >
        <X size={15} />
      </ControlButton>
    </div>
  );
}

function ControlButton({
  label,
  onClick,
  danger = false,
  children,
}: {
  label: string;
  onClick: () => void;
  danger?: boolean;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={label}
      aria-label={label}
      className={`flex h-8 w-[46px] items-center justify-center text-fg-muted transition-colors focus-visible:outline-none focus-visible:shadow-[0_0_0_2px_var(--color-background),0_0_0_4px_var(--color-accent)] ${
        danger
          ? "hover:bg-danger hover:text-white"
          : "hover:bg-hover hover:text-fg"
      }`}
    >
      {children}
    </button>
  );
}
