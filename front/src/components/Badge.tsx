import { has } from "../i18n";
import { useT } from "../hooks";

/** Cognito UserStatus values we have wording for; anything else shows raw. */
const VARIANTS: Record<string, string> = {
  CONFIRMED: "ok",
  UNCONFIRMED: "warn",
  FORCE_CHANGE_PASSWORD: "warn",
  RESET_REQUIRED: "warn",
  COMPROMISED: "off",
};

export function StatusBadge({ status }: { status: string | null }) {
  const t = useT();
  if (!status) return <span className="badge">—</span>;
  const key = `status.${status}`;
  const variant = VARIANTS[status];
  return (
    <span className={variant ? `badge badge--${variant}` : "badge"}>
      {has(key) ? t(key) : status}
    </span>
  );
}

export function EnabledBadge({ enabled }: { enabled: boolean }) {
  const t = useT();
  return (
    <span className={`badge badge--${enabled ? "ok" : "off"}`}>
      {t(enabled ? "user.enabled" : "user.disabled")}
    </span>
  );
}
