import { useEffect, useState, type ReactNode } from "react";
import { useT } from "../hooks";
import { has } from "../i18n";
import type { MfaPreference, TotpSetup } from "../types";

/** Cognito's own names for the factors, as they appear in UserMFASettingList. */
export const FACTORS = ["SMS_MFA", "SOFTWARE_TOKEN_MFA", "EMAIL_OTP"] as const;
type Factor = (typeof FACTORS)[number];

/** The checkboxes, before they are saved. */
type Draft = Record<Factor, boolean>;

const seed = (enabled: string[]): Draft => ({
  SMS_MFA: enabled.includes("SMS_MFA"),
  SOFTWARE_TOKEN_MFA: enabled.includes("SOFTWARE_TOKEN_MFA"),
  EMAIL_OTP: enabled.includes("EMAIL_OTP"),
});

/**
 * The factors a user has on, in the wording of the catalog. Cognito may name
 * one we have no wording for, which shows raw rather than as a missing key.
 */
export function MfaSummary({
  enabled,
  preferred,
}: {
  enabled: string[];
  preferred: string | null;
}) {
  const t = useT();
  const label = (factor: string) =>
    has(`mfa.${factor}`) ? t(`mfa.${factor}`) : factor;

  if (enabled.length === 0) return <>{t("account.mfaOff")}</>;
  return (
    <>
      {enabled.map(label).join(" / ")}
      {preferred && ` (${t("detail.preferred")}: ${label(preferred)})`}
    </>
  );
}

interface Props {
  /** Factors Cognito currently has switched on for the user. */
  enabled: string[];
  preferred: string | null;
  /** The pool's own MFA setting: "OFF", "ON" or "OPTIONAL". */
  poolMfa: string;
  busy: boolean;
  hint?: string;
  onSave: (preference: MfaPreference) => void;
  /** Enrolment or removal actions, shown under the form. */
  children?: ReactNode;
}

export function MfaCard({
  enabled,
  preferred,
  poolMfa,
  busy,
  hint,
  onSave,
  children,
}: Props) {
  const t = useT();
  const [draft, setDraft] = useState<Draft>(() => seed(enabled));
  const [prefer, setPrefer] = useState(preferred ?? "");

  // Re-seeded whenever the loaded settings change, so a save leaves the form
  // showing what Cognito now holds rather than what was asked for.
  useEffect(() => {
    setDraft(seed(enabled));
    setPrefer(preferred ?? "");
  }, [enabled.join(","), preferred]);

  const toggle = (factor: Factor, on: boolean) => {
    setDraft((current) => ({ ...current, [factor]: on }));
    // A factor that is off cannot be the preferred one, and the server says so
    // rather than guessing; clearing it here keeps the form from asking.
    if (!on && prefer === factor) setPrefer("");
  };

  const save = (event: React.FormEvent) => {
    event.preventDefault();
    // Only the factors in play are named. A pool that never enabled, say, email
    // MFA rejects a request that mentions it at all, even to switch it off.
    const involved = (factor: Factor) => draft[factor] || enabled.includes(factor);
    onSave({
      preferred: prefer || null,
      ...(involved("SMS_MFA") ? { sms: draft.SMS_MFA } : {}),
      ...(involved("SOFTWARE_TOKEN_MFA") ? { softwareToken: draft.SOFTWARE_TOKEN_MFA } : {}),
      ...(involved("EMAIL_OTP") ? { email: draft.EMAIL_OTP } : {}),
    });
  };

  return (
    <div className="card">
      <h2>{t("account.mfa")}</h2>
      {poolMfa === "OFF" && <p className="alert alert--warn">{t("mfa.poolOff")}</p>}
      {hint && <p className="hint">{hint}</p>}
      <dl className="dl">
        <dt>{t("mfa.current")}</dt>
        <dd>
          <MfaSummary enabled={enabled} preferred={preferred} />
        </dd>
      </dl>
      <form onSubmit={save}>
        {FACTORS.map((factor) => (
          <label className="check" key={factor}>
            <input
              type="checkbox"
              checked={draft[factor]}
              onChange={(event) => toggle(factor, event.target.checked)}
            />
            <span>{t(`mfa.${factor}`)}</span>
          </label>
        ))}
        <label className="field field--tight">
          <span className="field__label">{t("mfa.preferred")}</span>
          <select value={prefer} onChange={(event) => setPrefer(event.target.value)}>
            <option value="">{t("mfa.noPreference")}</option>
            {FACTORS.filter((factor) => draft[factor]).map((factor) => (
              <option key={factor} value={factor}>
                {t(`mfa.${factor}`)}
              </option>
            ))}
          </select>
        </label>
        <button type="submit" className="btn btn--primary" disabled={busy}>
          {t("common.save")}
        </button>
      </form>
      {children}
    </div>
  );
}

interface TotpProps {
  busy: boolean;
  /** The secret to enrol with, or null when the request failed. */
  onStart: () => Promise<TotpSetup | null>;
  /** Whether the code was accepted; a rejected one keeps the form open. */
  onVerify: (code: string, deviceName: string) => Promise<boolean>;
}

/**
 * Authenticator-app enrolment. Nothing changes until a code from the new
 * secret comes back, so an abandoned setup leaves the account as it was.
 */
export function TotpSetupBlock({ busy, onStart, onVerify }: TotpProps) {
  const t = useT();
  const [setup, setSetup] = useState<TotpSetup | null>(null);
  const [code, setCode] = useState("");
  const [device, setDevice] = useState("");

  if (!setup) {
    return (
      <>
        <hr />
        <div className="row row--gap">
          <button
            type="button"
            className="btn"
            disabled={busy}
            onClick={() => void onStart().then(setSetup)}
          >
            {t("mfa.setUpTotp")}
          </button>
          <span className="hint">{t("mfa.setUpTotpHint")}</span>
        </div>
      </>
    );
  }

  const close = () => {
    setSetup(null);
    setCode("");
    setDevice("");
  };

  return (
    <>
      <hr />
      <h3>{t("mfa.setUpTotp")}</h3>
      <p className="hint">{t("mfa.scanHint")}</p>
      {setup.qrDataUri && <img className="qr" src={setup.qrDataUri} alt={t("mfa.qrAlt")} />}
      <dl className="dl">
        <dt>{t("mfa.secret")}</dt>
        <dd className="mono">{setup.secretCode}</dd>
      </dl>
      <form
        onSubmit={(event) => {
          event.preventDefault();
          void onVerify(code, device).then((done) => done && close());
        }}
      >
        <div className="grid-2">
          <label className="field">
            <span className="field__label">{t("account.code")}</span>
            <input
              type="text"
              value={code}
              inputMode="numeric"
              autoComplete="one-time-code"
              required
              onChange={(event) => setCode(event.target.value)}
            />
          </label>
          <label className="field">
            <span className="field__label">{t("mfa.deviceName")}</span>
            <input
              type="text"
              value={device}
              autoComplete="off"
              placeholder={t("mfa.deviceNamePlaceholder")}
              onChange={(event) => setDevice(event.target.value)}
            />
          </label>
        </div>
        <div className="row row--gap">
          <button type="submit" className="btn btn--primary" disabled={busy}>
            {t("mfa.confirmTotp")}
          </button>
          <button type="button" className="btn" onClick={close}>
            {t("common.cancel")}
          </button>
        </div>
      </form>
    </>
  );
}
