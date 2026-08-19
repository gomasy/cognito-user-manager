import { useEffect, useState } from "react";
import { api } from "../api";
import { has } from "../i18n";
import { useT } from "../hooks";
import type { Challenge, PublicInfo } from "../types";

const CODE_CHALLENGES = ["SMS_MFA", "EMAIL_OTP", "SOFTWARE_TOKEN_MFA"];

export function Login({ onSignedIn }: { onSignedIn: () => void }) {
  const t = useT();
  const [info, setInfo] = useState<PublicInfo | null>(null);
  const [challenge, setChallenge] = useState<Challenge | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [code, setCode] = useState("");
  const [mfaType, setMfaType] = useState("");
  const [attributes, setAttributes] = useState<Record<string, string>>({});

  useEffect(() => {
    api.publicInfo().then(setInfo).catch(() => setInfo(null));
  }, []);

  const label = (name: string) => {
    const key = `challenge.${name}`;
    return has(key) ? t(key) : name;
  };

  async function submit(action: () => Promise<void>) {
    setBusy(true);
    setError("");
    try {
      await action();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  const signIn = (event: React.FormEvent) => {
    event.preventDefault();
    void submit(async () => {
      const outcome = await api.login(username, password);
      if (outcome.status === "signedIn") onSignedIn();
      else {
        setChallenge(outcome.challenge);
        setMfaType(outcome.challenge.mfaOptions[0] ?? "");
      }
    });
  };

  const answer = (event: React.FormEvent) => {
    event.preventDefault();
    void submit(async () => {
      const outcome = await api.answerChallenge({
        newPassword,
        confirmPassword,
        code,
        mfaType,
        userAttributes: attributes,
      });
      if (outcome.status === "signedIn") onSignedIn();
      else {
        // Answering one challenge can surface the next one.
        setChallenge(outcome.challenge);
        setCode("");
        setNewPassword("");
        setConfirmPassword("");
      }
    });
  };

  const restart = () => {
    setChallenge(null);
    setError("");
    setPassword("");
  };

  return (
    <main className="auth">
      <form className="auth__card" onSubmit={challenge ? answer : signIn}>
        <h1>{challenge ? label(challenge.name) : t("login.title")}</h1>
        <p className="hint">
          {challenge
            ? challenge.name === "NEW_PASSWORD_REQUIRED"
              ? t("login.newPasswordHint")
              : challenge.destination
                ? t("login.codeSentTo", { destination: challenge.destination })
                : t("login.challengeHint")
            : info?.poolName
              ? t("login.pool", { pool: info.poolName })
              : t("login.hint")}
        </p>

        {error && <p className="alert alert--error">{error}</p>}

        {!challenge && (
          <>
            <label className="field">
              <span className="field__label">{t("login.username")}</span>
              <input
                type="text"
                value={username}
                autoComplete="username"
                required
                autoFocus
                onChange={(event) => setUsername(event.target.value)}
              />
            </label>
            <label className="field">
              <span className="field__label">{t("login.password")}</span>
              <input
                type="password"
                value={password}
                autoComplete="current-password"
                required
                onChange={(event) => setPassword(event.target.value)}
              />
            </label>
          </>
        )}

        {challenge?.name === "NEW_PASSWORD_REQUIRED" && (
          <>
            <label className="field">
              <span className="field__label">{t("login.newPassword")}</span>
              <input
                type="password"
                value={newPassword}
                autoComplete="new-password"
                required
                autoFocus
                onChange={(event) => setNewPassword(event.target.value)}
              />
            </label>
            <label className="field">
              <span className="field__label">{t("login.confirmPassword")}</span>
              <input
                type="password"
                value={confirmPassword}
                autoComplete="new-password"
                required
                onChange={(event) => setConfirmPassword(event.target.value)}
              />
            </label>
            {challenge.requiredAttributes.map((name) => (
              <label className="field" key={name}>
                <span className="field__label">
                  {name}
                  <span className="field__required">*</span>
                </span>
                <input
                  type="text"
                  value={attributes[name] ?? ""}
                  required
                  onChange={(event) =>
                    setAttributes((current) => ({ ...current, [name]: event.target.value }))
                  }
                />
              </label>
            ))}
          </>
        )}

        {challenge?.name === "SELECT_MFA_TYPE" && (
          <label className="field">
            <span className="field__label">{t("login.mfaType")}</span>
            <select value={mfaType} onChange={(event) => setMfaType(event.target.value)}>
              {challenge.mfaOptions.map((option) => (
                <option key={option} value={option}>
                  {label(option)}
                </option>
              ))}
            </select>
          </label>
        )}

        {challenge && CODE_CHALLENGES.includes(challenge.name) && (
          <label className="field">
            <span className="field__label">{t("login.code")}</span>
            <input
              type="text"
              value={code}
              inputMode="numeric"
              autoComplete="one-time-code"
              required
              autoFocus
              onChange={(event) => setCode(event.target.value)}
            />
          </label>
        )}

        <div className="row row--gap">
          <button type="submit" className="btn btn--primary" disabled={busy}>
            {busy ? t("common.working") : challenge ? t("common.continue") : t("login.submit")}
          </button>
          {challenge && (
            <button type="button" className="btn" onClick={restart}>
              {t("login.restart")}
            </button>
          )}
        </div>

        {info && <p className="auth__version">{info.version}</p>}
      </form>
    </main>
  );
}
