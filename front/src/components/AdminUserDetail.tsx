import { useCallback, useEffect, useState } from "react";
import { api } from "../api";
import { errorText, useAction, useDateFormat, useNavigate, useT, useToast } from "../hooks";
import type { AttributeField, UserDetail } from "../types";
import { AttributeFields, initialDraft, toPatch, type Draft } from "./AttributeFields";
import { EnabledBadge, StatusBadge } from "./Badge";
import { MfaCard, MfaSummary } from "./Mfa";

interface Props {
  username: string;
  fields: AttributeField[];
  editable: AttributeField[];
  groups: string[];
  /** The pool's own MFA setting, so the card can say when it is off. */
  poolMfa: string;
  isSelf: (username: string) => boolean;
}

export function AdminUserDetail({
  username,
  fields,
  editable,
  groups,
  poolMfa,
  isSelf,
}: Props) {
  const t = useT();
  const navigate = useNavigate();
  const formatDate = useDateFormat();
  const { notify } = useToast();

  const [user, setUser] = useState<UserDetail | null>(null);
  const [missing, setMissing] = useState(false);
  const [draft, setDraft] = useState<Draft>({});
  const [selectedGroups, setSelectedGroups] = useState<string[]>([]);

  const load = useCallback(async () => {
    try {
      const detail = await api.user(username);
      setUser(detail);
      setDraft(initialDraft(fields, detail.attributes));
      setSelectedGroups(detail.groups);
    } catch (e) {
      setMissing(true);
      notify(errorText(e), "error");
    }
  }, [username, fields, notify]);

  useEffect(() => {
    void load();
  }, [load]);

  const { busy, run } = useAction(load);

  if (missing) {
    return (
      <main className="page page--narrow">
        <div className="card">
          <h2>{t("detail.notFound")}</h2>
          <button type="button" className="btn" onClick={() => navigate("/admin")}>
            {t("admin.backToList")}
          </button>
        </div>
      </main>
    );
  }

  if (!user) {
    return (
      <main className="page page--narrow">
        <p className="hint">{t("common.loading")}</p>
      </main>
    );
  }

  const self = isSelf(user.username);

  return (
    <main className="page page--narrow">
      <header className="page__header">
        <p className="hint">
          <a
            href="/admin"
            onClick={(event) => {
              event.preventDefault();
              navigate("/admin");
            }}
          >
            {t("admin.backToList")}
          </a>
        </p>
        <div className="page__header--split">
          <h1 className="mono">{user.username}</h1>
          <span className="row row--gap">
            <EnabledBadge enabled={user.enabled} />
            <StatusBadge status={user.status} />
          </span>
        </div>
      </header>

      <div className="card">
        <h2>{t("detail.overview")}</h2>
        <dl className="dl">
          <dt>{t("attr.sub")}</dt>
          <dd className="mono">{user.attributes.sub ?? "—"}</dd>
          <dt>{t("user.created")}</dt>
          <dd>{formatDate(user.createdAt)}</dd>
          <dt>{t("user.updated")}</dt>
          <dd>{formatDate(user.updatedAt)}</dd>
          <dt>{t("account.mfa")}</dt>
          <dd>
            <MfaSummary enabled={user.mfa} preferred={user.preferredMfa} />
          </dd>
        </dl>
      </div>

      <form
        className="card"
        onSubmit={(event) => {
          event.preventDefault();
          // Only mutable attributes are sent; the rest are shown read-only.
          void run(() => api.updateUser(user.username, toPatch(editable, draft)));
        }}
      >
        <h2>{t("detail.attributes")}</h2>
        <p className="hint">{t("account.clearHint")}</p>
        <AttributeFields
          fields={fields}
          draft={draft}
          onChange={(name, value) => setDraft((d) => ({ ...d, [name]: value }))}
        />
        <button type="submit" className="btn btn--primary" disabled={busy}>
          {t("common.save")}
        </button>
      </form>

      <form
        className="card"
        onSubmit={(event) => {
          event.preventDefault();
          void run(() => api.setGroups(user.username, selectedGroups));
        }}
      >
        <h2>{t("user.groups")}</h2>
        {groups.length === 0 ? (
          <p className="hint">{t("detail.noGroups")}</p>
        ) : (
          groups.map((group) => (
            <label className="check" key={group}>
              <input
                type="checkbox"
                checked={selectedGroups.includes(group)}
                onChange={(event) =>
                  setSelectedGroups((current) =>
                    event.target.checked
                      ? [...current, group]
                      : current.filter((name) => name !== group),
                  )
                }
              />
              <span>{group}</span>
            </label>
          ))
        )}
        <button type="submit" className="btn btn--primary" disabled={busy}>
          {t("detail.updateGroups")}
        </button>
      </form>

      <MfaCard
        enabled={user.mfa}
        preferred={user.preferredMfa}
        poolMfa={poolMfa}
        busy={busy}
        hint={t("mfa.adminHint")}
        onSave={(preference) => void run(() => api.setUserMfa(user.username, preference))}
      >
        <hr />
        <div className="row row--gap">
          <button
            type="button"
            className="btn btn--danger"
            disabled={busy}
            onClick={() => void run(() => api.deleteUserTotp(user.username))}
          >
            {t("mfa.removeTotp")}
          </button>
          <span className="hint">{t("mfa.removeTotpHint")}</span>
        </div>
      </MfaCard>

      <PasswordCard
        busy={busy}
        onSet={(password, permanent) =>
          run(() => api.setPassword(user.username, password, permanent), false)
        }
        onReset={() => run(() => api.resetPassword(user.username), false)}
      />

      <div className="card">
        <h2>{t("detail.sessions")}</h2>
        <div className="row row--gap">
          <button
            type="button"
            className="btn"
            disabled={busy}
            onClick={() => void run(() => api.signOutUser(user.username), false)}
          >
            {t("detail.signOutEverywhere")}
          </button>
          {user.status === "FORCE_CHANGE_PASSWORD" && (
            <button
              type="button"
              className="btn"
              disabled={busy}
              onClick={() => void run(() => api.resendInvite(user.username), false)}
            >
              {t("detail.resendInvite")}
            </button>
          )}
        </div>
      </div>

      <DangerZone
        user={user}
        self={self}
        busy={busy}
        onToggle={() => run(() => api.setEnabled(user.username, !user.enabled))}
        onDelete={async () => {
          // A refused delete leaves the user there, so the page has to stay too.
          if (await run(() => api.deleteUser(user.username), false)) navigate("/admin");
        }}
      />
    </main>
  );
}

function PasswordCard({
  busy,
  onSet,
  onReset,
}: {
  busy: boolean;
  onSet: (password: string, permanent: boolean) => void;
  onReset: () => void;
}) {
  const t = useT();
  const [password, setPassword] = useState("");
  const [permanent, setPermanent] = useState(false);

  return (
    <div className="card">
      <h2>{t("detail.password")}</h2>
      <form
        onSubmit={(event) => {
          event.preventDefault();
          onSet(password, permanent);
          setPassword("");
        }}
      >
        <label className="field">
          <span className="field__label">{t("detail.setPassword")}</span>
          <input
            type="text"
            value={password}
            autoComplete="off"
            required
            onChange={(event) => setPassword(event.target.value)}
          />
        </label>
        <label className="check">
          <input
            type="checkbox"
            checked={permanent}
            onChange={(event) => setPermanent(event.target.checked)}
          />
          <span>{t("detail.permanent")}</span>
        </label>
        <button type="submit" className="btn btn--primary" disabled={busy}>
          {t("detail.applyPassword")}
        </button>
      </form>
      <hr />
      <div className="row row--gap">
        <button type="button" className="btn" disabled={busy} onClick={onReset}>
          {t("detail.sendResetCode")}
        </button>
        <span className="hint">{t("detail.resetHint")}</span>
      </div>
    </div>
  );
}

function DangerZone({
  user,
  self,
  busy,
  onToggle,
  onDelete,
}: {
  user: UserDetail;
  self: boolean;
  busy: boolean;
  onToggle: () => void;
  onDelete: () => void;
}) {
  const t = useT();
  const [confirm, setConfirm] = useState("");
  const [open, setOpen] = useState(false);

  return (
    <div className="card card--danger">
      <h2>{t("detail.danger")}</h2>
      {self && <p className="alert alert--warn">{t("detail.selfWarning")}</p>}

      <div className="row row--gap">
        <button
          type="button"
          className={user.enabled ? "btn btn--danger" : "btn"}
          disabled={busy || (user.enabled && self)}
          onClick={onToggle}
        >
          {t(user.enabled ? "detail.disable" : "detail.enable")}
        </button>
        {!self && !open && (
          <button type="button" className="btn btn--danger" onClick={() => setOpen(true)}>
            {t("detail.delete")}
          </button>
        )}
      </div>

      {!self && open && (
        <form
          onSubmit={(event) => {
            event.preventDefault();
            onDelete();
          }}
        >
          <label className="field">
            <span className="field__label">
              {t("detail.confirmDelete", { username: user.username })}
            </span>
            <input
              type="text"
              value={confirm}
              autoComplete="off"
              required
              onChange={(event) => setConfirm(event.target.value)}
            />
          </label>
          <div className="row row--gap">
            <button
              type="submit"
              className="btn btn--danger"
              disabled={busy || confirm !== user.username}
            >
              {t("detail.deleteForever")}
            </button>
            <button type="button" className="btn" onClick={() => setOpen(false)}>
              {t("common.cancel")}
            </button>
          </div>
        </form>
      )}
    </div>
  );
}
