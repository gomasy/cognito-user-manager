import { useEffect, useState } from "react";
import { api } from "../api";
import { errorText, useT, useWording } from "../hooks";
import type { AuthOutcome, Challenge, PublicInfo } from "../types";
import * as webauthn from "../webauthn";

const CODE_CHALLENGES = ["SMS_MFA", "EMAIL_OTP", "SOFTWARE_TOKEN_MFA"];

export function Login({ onSignedIn }: { onSignedIn: () => void }) {
  const t = useT();
  const label = useWording("challenge");
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

  async function submit(action: () => Promise<void>) {
    setBusy(true);
    setError("");
    try {
      await action();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  }

  /**
   * Shows what the server answered with, and hands back the challenge that
   * came with it — answering one can surface the next.
   */
  const advance = (outcome: AuthOutcome): Challenge | null => {
    if (outcome.status === "signedIn") {
      onSignedIn();
      return null;
    }
    setChallenge(outcome.challenge);
    setMfaType(outcome.challenge.mfaOptions[0] ?? "");
    setCode("");
    setNewPassword("");
    setConfirmPassword("");
    return outcome.challenge;
  };

  const signIn = (event: React.FormEvent) => {
    event.preventDefault();
    void submit(async () => {
      advance(await api.login(username, password));
    });
  };

  /**
   * Signs the options Cognito issued and sends the assertion back. A cancelled
   * prompt can simply be tried again; the challenge is good until it expires.
   */
  const useThePasskey = async (current: Challenge) => {
    const options = current.credentialRequestOptions;
    if (!options) throw new Error(t("passkey.failed"));
    const credential = await webauthn.authenticate(JSON.parse(options));
    advance(await api.answerChallenge({ credential: JSON.stringify(credential) }));
  };

  const passkeySignIn = () => {
    void submit(async () => {
      const next = advance(await api.passkeyLogin(username));
      if (next?.name === "WEB_AUTHN") await useThePasskey(next);
    });
  };

  const answer = (event: React.FormEvent) => {
    event.preventDefault();
    if (challenge?.name === "WEB_AUTHN") {
      void submit(() => useThePasskey(challenge));
      return;
    }
    void submit(async () => {
      advance(
        await api.answerChallenge({
          newPassword,
          confirmPassword,
          code,
          mfaType,
          userAttributes: attributes,
        }),
      );
    });
  };

  // Offered only where it works: a pool that allows passkeys, a browser that
  // can hold one.
  const passkeyReady = (info?.passkeySignIn ?? false) && webauthn.isSupported();

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
              : challenge.name === "WEB_AUTHN"
                ? t("login.passkeyHint")
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
            {busy
              ? t("common.working")
              : challenge?.name === "WEB_AUTHN"
                ? t("login.usePasskey")
                : challenge
                  ? t("common.continue")
                  : t("login.submit")}
          </button>
          {!challenge && passkeyReady && (
            <button
              type="button"
              className="btn"
              disabled={busy || !username.trim()}
              onClick={passkeySignIn}
            >
              {t("login.passkey")}
            </button>
          )}
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
