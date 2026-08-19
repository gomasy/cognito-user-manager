import { useState } from "react";
import { api } from "../api";
import { useNavigate, useT, useToast } from "../hooks";
import type { AttributeField } from "../types";
import { AttributeFields, initialDraft, toPatch, type Draft } from "./AttributeFields";

interface Props {
  fields: AttributeField[];
  groups: string[];
  usernameIsEmail: boolean;
}

export function AdminUserCreate({ fields, groups, usernameIsEmail }: Props) {
  const t = useT();
  const navigate = useNavigate();
  const { notify } = useToast();

  const [username, setUsername] = useState("");
  const [draft, setDraft] = useState<Draft>(() => initialDraft(fields, {}));
  const [temporaryPassword, setTemporaryPassword] = useState("");
  const [suppressMessage, setSuppressMessage] = useState(false);
  const [selected, setSelected] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    setBusy(true);
    try {
      const created = await api.createUser({
        username,
        attributes: toPatch(fields, draft),
        temporaryPassword,
        suppressMessage,
        groups: selected,
      });
      notify(created.message);
      navigate(`/admin/users/${encodeURIComponent(created.username)}`);
    } catch (e) {
      notify(e instanceof Error ? e.message : String(e), "error");
    } finally {
      setBusy(false);
    }
  };

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
        <h1>{t("create.title")}</h1>
      </header>

      <form onSubmit={submit}>
        <div className="card">
          <h2>{t("create.basics")}</h2>
          <label className="field">
            <span className="field__label">
              {t("user.username")}
              <span className="field__required">*</span>
            </span>
            <input
              type={usernameIsEmail ? "email" : "text"}
              value={username}
              required
              autoFocus
              autoComplete="off"
              onChange={(event) => setUsername(event.target.value)}
            />
            <span className="hint">
              {usernameIsEmail ? t("create.usernameEmail") : t("create.usernameFixed")}
            </span>
          </label>
          <AttributeFields
            fields={fields}
            draft={draft}
            allowImmutable
            onChange={(name, value) => setDraft((d) => ({ ...d, [name]: value }))}
          />
        </div>

        <div className="card">
          <h2>{t("create.initialPassword")}</h2>
          <label className="field">
            <span className="field__label">{t("create.temporaryPassword")}</span>
            <input
              type="text"
              value={temporaryPassword}
              autoComplete="off"
              onChange={(event) => setTemporaryPassword(event.target.value)}
            />
            <span className="hint">{t("create.temporaryHint")}</span>
          </label>
          <label className="check">
            <input
              type="checkbox"
              checked={suppressMessage}
              onChange={(event) => setSuppressMessage(event.target.checked)}
            />
            <span>{t("create.suppress")}</span>
          </label>
        </div>

        {groups.length > 0 && (
          <div className="card">
            <h2>{t("user.groups")}</h2>
            {groups.map((group) => (
              <label className="check" key={group}>
                <input
                  type="checkbox"
                  checked={selected.includes(group)}
                  onChange={(event) =>
                    setSelected((current) =>
                      event.target.checked
                        ? [...current, group]
                        : current.filter((name) => name !== group),
                    )
                  }
                />
                <span>{group}</span>
              </label>
            ))}
          </div>
        )}

        <div className="row row--gap">
          <button type="submit" className="btn btn--primary" disabled={busy}>
            {busy ? t("common.working") : t("create.submit")}
          </button>
          <button
            type="button"
            className="btn"
            onClick={() => navigate("/admin")}
          >
            {t("common.cancel")}
          </button>
        </div>
      </form>
    </main>
  );
}
