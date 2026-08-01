import { type ComponentProps } from "react";
import { clsx } from "clsx";

/**
 * The focus ring, as one string in one module.
 *
 * The inner ring is `--color-background`, fixed — never the parent's fill. It
 * used to be hand-picked per call site (`--color-background`, `--color-surface-2`
 * and `--color-surface-3` all appeared), which means every one of them is wrong
 * the moment a surface changes.
 *
 * Exported because inputs, selects and tabs need the same ring and are not
 * buttons. Box-shadow rather than outline, per the house rule, and never inside
 * a `transition-*` property list: the ring must appear instantly or it reads as
 * input lag on the one signal that has to feel immediate.
 */
export const FOCUS = "focus-visible:outline-none focus-visible:shadow-focus";
export const FOCUS_DANGER =
  "focus-visible:outline-none focus-visible:shadow-focus-danger";

/** Everything shared. The transition list names its properties so that
 *  `box-shadow` — the focus ring — is excluded by construction. */
const BASE =
  "inline-flex select-none items-center justify-center gap-2 whitespace-nowrap font-medium " +
  "transition-[color,background-color,border-color,filter] " +
  "disabled:pointer-events-none disabled:opacity-50 " +
  "[&_svg]:size-4 [&_svg]:shrink-0";

/** Controls get a height, not a vertical padding — that is what killed the
 *  padding zoo. Radius scales with the control: 4px on a 24px chip, 8px from
 *  32px up. Icon buttons are one geometry, no exceptions. */
const SIZE = {
  xs: "h-6 rounded-xs px-2 text-xs",
  sm: "h-8 rounded-md px-3 text-sm",
  md: "h-9 rounded-md px-4 text-sm",
  icon: "h-8 w-8 rounded-md",
} as const;

/** `link` carries no colour of its own: it inherits from the sentence it sits
 *  in, so the same component reads red inside the error banner and accent
 *  inside the update banner without a per-variant override fighting Tailwind's
 *  emission order. */
const VARIANT = {
  primary: `bg-accent text-background hover:brightness-110 ${FOCUS}`,
  secondary: `bg-surface-3 text-fg hover:bg-hover ${FOCUS}`,
  ghost: `text-fg-muted hover:bg-surface-3 hover:text-fg ${FOCUS}`,
  danger: `bg-danger text-background hover:brightness-110 ${FOCUS_DANGER}`,
  link: `rounded-xs underline underline-offset-2 ${FOCUS}`,
} as const;

interface Props extends ComponentProps<"button"> {
  variant?: keyof typeof VARIANT;
  size?: keyof typeof SIZE;
}

/**
 * The one button.
 *
 * The app had 33 buttons, 20 of them with no focus ring, the same accent
 * primary written five different ways and `disabled:opacity` at 30, 40 and 50.
 * Every one of those is a property of this file now.
 *
 * `link` skips the size classes entirely rather than overriding them: `h-6` and
 * `h-auto` are the same CSS property, and which one wins depends on Tailwind's
 * emission order, not on the order of the class attribute.
 */
export function Button({
  variant = "primary",
  size = "sm",
  className,
  type = "button",
  ...props
}: Props) {
  return (
    <button
      type={type}
      className={clsx(
        BASE,
        VARIANT[variant],
        variant !== "link" && SIZE[size],
        className,
      )}
      {...props}
    />
  );
}
