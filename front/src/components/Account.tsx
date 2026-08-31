import { useEffect, useState } from "react";
import { api } from "../api";
import { errorText, useAction, useLabel, useT, useToast } from "../hooks";
import type { AttributeField, MyProfile } from "../types";
import { AttributeFields, initialDraft, toPatch, type Draft } from "./AttributeFields";
import { MfaCard, MfaSummary, TotpSetupBlock } from "./Mfa";

const CONTACTS = [
  { name: "email", verified: "email_verified" },
  { name: "phone_number", verified: "phone_number_verified" },
];

interface Props {
  fields: AttributeField[];
  /** The pool's own MFA setting, so the card can say when it is off. */
  poolMfa: string;
}

export function Account({ fields, poolMfa }: Props) {
  const t = useT();
  const label = useLabel();
  const { notify } = useToast();
  const [profile, setProfile] = useState<MyProfile | null>(null);
  const [failed, setFailed] = useState(false);
  const [draft, setDraft] = useState<Draft>({});

  const load = async () => {
    const loaded = await api.profile();
    setProfile(loaded);
    setFailed(false);
    setDraft(initialDraft(fields, loaded.attributes));
  };

  // The toast expires; without a profile there is nothing else on the page,
  // which would leave it on "loading" for good.
  const start = () =>
    load().catch((e) => {
      setFailed(true);
      notify(errorText(e), "error");
    });

  useEffect(() => {
    // Loads once: re-running on a field-list change would discard edits in progress.
    void start();
  }, []);

  const { busy, run, attempt } = useAction(load);

  // Enrolment answers with the secret rather than a sentence, so it takes the
  // other half of the hook: the toast only comes once a code has confirmed it.
  const startTotp = () => attempt(() => api.startTotp());
  const verifyTotp = (code: string, deviceName: string) =>
    run(() => api.verifyTotp(code, deviceName || undefined));

  if (!profile) {
    return (
      <main className="page page--narrow">
        {failed ? (
          <div className="card">
            <h2>{t("common.loadFailed")}</h2>
            <button type="button" className="btn" onClick={() => void start()}>
              {t("common.retry")}
            </button>
          </div>
        ) : (
          <p className="hint">{t("common.loading")}</p>
        )}
      </main>
    );
  }

  const unverified = CONTACTS.filter(
    (contact) =>
      profile.attributes[contact.name] && profile.attributes[contact.verified] !== "true",
  );

  return (
    <main className="page page--narrow">
      <header className="page__header">
        <h1>{t("account.title")}</h1>
        <p className="mono">{profile.username}</p>
      </header>

      <form
        className="card"
        onSubmit={(event) => {
          event.preventDefault();
          void run(() => api.updateProfile(toPatch(fields, draft)));
        }}
      >
        <h2>{t("account.profile")}</h2>
        <p className="hint">{t("account.clearHint")}</p>
        {fields.length === 0 ? (
          <p className="hint">{t("account.noEditable")}</p>
        ) : (
          <AttributeFields
            fields={fields}
            draft={draft}
            onChange={(name, value) => setDraft((d) => ({ ...d, [name]: value }))}
          />
        )}
        <button type="submit" className="btn btn--primary" disabled={busy}>
          {t("common.save")}
        </button>
      </form>

      {unverified.length > 0 && (
        <div className="card">
          <h2>{t("account.unverified")}</h2>
          <p className="hint">{t("account.unverifiedHint")}</p>
          {unverified.map((contact) => (
            <VerifyRow
              key={contact.name}
              name={contact.name}
              label={label(contact.name)}
              busy={busy}
              onSend={() => run(() => api.sendVerificationCode(contact.name), false)}
              onVerify={(code) => run(() => api.verifyAttribute(contact.name, code))}
            />
          ))}
        </div>
      )}

      <PasswordCard busy={busy} onSubmit={(a, b, c) => run(() => api.changePassword(a, b, c), false)} />

      <MfaCard
        enabled={profile.mfa}
        preferred={profile.preferredMfa}
        poolMfa={poolMfa}
        busy={busy}
        onSave={(preference) => void run(() => api.setMyMfa(preference))}
      >
        <TotpSetupBlock busy={busy} onStart={startTotp} onVerify={verifyTotp} />
      </MfaCard>

      <div className="card">
        <h2>{t("account.info")}</h2>
        <dl className="dl">
          <dt>{t("account.username")}</dt>
          <dd className="mono">{profile.username}</dd>
          <dt>{t("account.groups")}</dt>
          <dd>
            {profile.groups.length === 0
              ? t("common.none")
              : profile.groups.map((group) => (
                  <span className="tag" key={group}>
                    {group}
                  </span>
                ))}
          </dd>
          <dt>{t("account.mfa")}</dt>
          <dd>
            <MfaSummary enabled={profile.mfa} preferred={profile.preferredMfa} />
          </dd>
        </dl>
      </div>
    </main>
  );
}

function VerifyRow({
  name,
  label,
  busy,
  onSend,
  onVerify,
}: {
  name: string;
  label: string;
  busy: boolean;
  onSend: () => void;
  onVerify: (code: string) => void;
}) {
  const t = useT();
  const [code, setCode] = useState("");

  return (
    <div className="verify">
      <h3>
        {label} <span className="badge badge--warn">{t("user.unverified")}</span>
      </h3>
      <div className="row row--gap">
        <button type="button" className="btn" disabled={busy} onClick={onSend}>
          {t("account.sendCode")}
        </button>
        <form
          className="row row--gap"
          onSubmit={(event) => {
            event.preventDefault();
            onVerify(code);
            setCode("");
          }}
        >
          <input
            type="text"
            name={`${name}-code`}
            value={code}
            inputMode="numeric"
            placeholder={t("account.code")}
            required
            onChange={(event) => setCode(event.target.value)}
          />
          <button type="submit" className="btn btn--primary" disabled={busy}>
            {t("account.verify")}
          </button>
        </form>
      </div>
    </div>
  );
}

function PasswordCard({
  busy,
  onSubmit,
}: {
  busy: boolean;
  onSubmit: (current: string, next: string, confirm: string) => void;
}) {
  const t = useT();
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [confirm, setConfirm] = useState("");

  return (
    <form
      className="card"
      onSubmit={(event) => {
        event.preventDefault();
        onSubmit(current, next, confirm);
        setCurrent("");
        setNext("");
        setConfirm("");
      }}
    >
      <h2>{t("account.changePassword")}</h2>
      <div className="grid-2">
        <label className="field">
          <span className="field__label">{t("account.currentPassword")}</span>
          <input
            type="password"
            value={current}
            autoComplete="current-password"
            required
            onChange={(event) => setCurrent(event.target.value)}
          />
        </label>
        <label className="field">
          <span className="field__label">{t("login.newPassword")}</span>
          <input
            type="password"
            value={next}
            autoComplete="new-password"
            required
            onChange={(event) => setNext(event.target.value)}
          />
        </label>
        <label className="field">
          <span className="field__label">{t("login.confirmPassword")}</span>
          <input
            type="password"
            value={confirm}
            autoComplete="new-password"
            required
            onChange={(event) => setConfirm(event.target.value)}
          />
        </label>
      </div>
      <button type="submit" className="btn btn--primary" disabled={busy}>
        {t("account.changePassword")}
      </button>
    </form>
  );
}
