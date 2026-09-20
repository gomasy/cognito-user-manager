import { useCallback, useEffect, useState } from "react";
import { api } from "../api";
import { useAction, useDateFormat, useT, useToast, useWording } from "../hooks";
import type { PasskeyCredential } from "../types";
import * as webauthn from "../webauthn";

interface Props {
  /**
   * Whether a passkey can be registered here. False leaves the card listing and
   * removing the ones a user already has, which the pool alone allows.
   */
  usable: boolean;
}

/**
 * The passkeys on the caller's own account. Registering is two calls around a
 * prompt only the browser can answer, and nothing is stored until the second
 * one goes through.
 */
export function PasskeyCard({ usable }: Props) {
  const t = useT();
  const label = useWording("passkey");
  const formatDate = useDateFormat();
  const { fail } = useToast();
  const [credentials, setCredentials] = useState<PasskeyCredential[] | null>(null);
  const [failed, setFailed] = useState(false);

  const load = useCallback(async () => {
    setCredentials(await api.passkeys());
    setFailed(false);
  }, []);

  // The toast expires, and a card left saying "loading" would be all that is
  // left of it.
  const start = () =>
    load().catch((e) => {
      setFailed(true);
      fail(e);
    });

  useEffect(() => {
    void start();
  }, []);

  const { busy, run } = useAction(load);

  const register = () =>
    void run(async () => {
      const options = await api.startPasskey();
      return api.addPasskey(await webauthn.create(options));
    });

  const describe = (credential: PasskeyCredential) =>
    [
      t("passkey.added", { date: formatDate(credential.createdAt) }),
      credential.attachment ? label(credential.attachment) : null,
      credential.transports.join(" / ") || null,
    ]
      .filter(Boolean)
      .join(" · ");

  // The two reasons registering cannot go ahead, in the order a user can act on.
  const blocked = !webauthn.isSupported()
    ? "passkey.unsupported"
    : !usable
      ? "passkey.registerDisabled"
      : null;

  return (
    <div className="card">
      <h2>{t("account.passkeys")}</h2>
      <p className="hint">{t("account.passkeysHint")}</p>

      {failed ? (
        <p className="hint">{t("common.loadFailed")}</p>
      ) : credentials === null ? (
        <p className="hint">{t("common.loading")}</p>
      ) : credentials.length === 0 ? (
        <p className="hint">{t("passkey.none")}</p>
      ) : (
        <ul className="passkeys">
          {credentials.map((credential) => (
            <li key={credential.id}>
              <div>
                <strong>{credential.name}</strong>
                <p className="passkeys__meta">{describe(credential)}</p>
              </div>
              <button
                type="button"
                className="btn btn--danger"
                disabled={busy}
                onClick={() => void run(() => api.deletePasskey(credential.id))}
              >
                {t("passkey.remove")}
              </button>
            </li>
          ))}
        </ul>
      )}

      {blocked && <p className="alert alert--warn">{t(blocked)}</p>}

      <div className="row row--gap">
        <button
          type="button"
          className="btn btn--primary"
          disabled={busy || blocked !== null}
          onClick={register}
        >
          {t("passkey.register")}
        </button>
        {failed && (
          <button type="button" className="btn" disabled={busy} onClick={() => void start()}>
            {t("common.retry")}
          </button>
        )}
      </div>
    </div>
  );
}
