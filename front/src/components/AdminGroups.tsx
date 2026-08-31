import { useCallback, useEffect, useState } from "react";
import { api } from "../api";
import { errorText, useAction, useDateFormat, useNavigate, useT, useToast } from "../hooks";
import type { GroupInfo } from "../types";

interface Props {
  /** The group that grants access to this console, or null if unknown. */
  adminGroup: string | null;
  /** Lets the shell refresh the group list the user screens assign from. */
  onGroupsChanged: () => void;
}

export function AdminGroups({ adminGroup, onGroupsChanged }: Props) {
  const t = useT();
  const navigate = useNavigate();
  const formatDate = useDateFormat();
  const { notify } = useToast();

  const [groups, setGroups] = useState<GroupInfo[] | null>(null);

  const load = useCallback(async () => {
    try {
      setGroups(await api.groups());
    } catch (e) {
      // Leaving this null would sit on "loading" with nothing in flight.
      setGroups([]);
      notify(errorText(e), "error");
    }
  }, [notify]);

  useEffect(() => {
    void load();
  }, [load]);

  const { busy, run } = useAction(load);

  const create = async (payload: {
    name: string;
    description: string | null;
    precedence: number | null;
  }) => {
    const created = await run(() => api.createGroup(payload));
    // The user screens assign from the list loaded at boot, which a new group
    // is not in yet.
    if (created) onGroupsChanged();
    return created;
  };

  const open = (name: string) => navigate(`/admin/groups/${encodeURIComponent(name)}`);

  return (
    <main className="page">
      <header className="page__header">
        <h1>{t("groups.title")}</h1>
        <p className="hint">{t("groups.hint")}</p>
      </header>

      <CreateCard busy={busy} onCreate={create} />

      <div className="table-wrap">
        <table>
          <thead>
            <tr>
              <th>{t("groups.name")}</th>
              <th>{t("groups.description")}</th>
              <th>{t("groups.precedence")}</th>
              <th>{t("user.created")}</th>
            </tr>
          </thead>
          <tbody>
            {(groups ?? []).map((group) => (
              <tr key={group.name}>
                <td className="mono">
                  <a
                    href={`/admin/groups/${encodeURIComponent(group.name)}`}
                    onClick={(event) => {
                      event.preventDefault();
                      open(group.name);
                    }}
                  >
                    {group.name}
                  </a>
                  {group.name === adminGroup && (
                    <span className="badge badge--ok">{t("nav.admin")}</span>
                  )}
                </td>
                <td>{group.description || "—"}</td>
                <td>{group.precedence ?? "—"}</td>
                <td>{formatDate(group.createdAt)}</td>
              </tr>
            ))}
          </tbody>
        </table>
        {groups === null && <p className="empty">{t("common.loading")}</p>}
        {groups?.length === 0 && <p className="empty">{t("groups.none")}</p>}
      </div>
    </main>
  );
}

function CreateCard({
  busy,
  onCreate,
}: {
  busy: boolean;
  onCreate: (payload: {
    name: string;
    description: string | null;
    precedence: number | null;
  }) => Promise<boolean>;
}) {
  const t = useT();
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [precedence, setPrecedence] = useState("");

  return (
    <form
      className="card"
      onSubmit={(event) => {
        event.preventDefault();
        void onCreate({
          name,
          description: description.trim() || null,
          // Left empty, Cognito assigns no precedence at all, which sorts the
          // group last rather than first.
          precedence: precedence === "" ? null : Number(precedence),
        }).then((done) => {
          if (!done) return;
          setName("");
          setDescription("");
          setPrecedence("");
        });
      }}
    >
      <h2>{t("groups.create")}</h2>
      <p className="hint">{t("groups.createHint")}</p>
      <div className="grid-2">
        <label className="field">
          <span className="field__label">
            {t("groups.name")} <span className="field__required">*</span>
          </span>
          <input
            type="text"
            value={name}
            autoComplete="off"
            required
            onChange={(event) => setName(event.target.value)}
          />
        </label>
        <label className="field">
          <span className="field__label">{t("groups.precedence")}</span>
          <input
            type="number"
            value={precedence}
            min={0}
            onChange={(event) => setPrecedence(event.target.value)}
          />
        </label>
        <label className="field field--grow">
          <span className="field__label">{t("groups.description")}</span>
          <input
            type="text"
            value={description}
            onChange={(event) => setDescription(event.target.value)}
          />
        </label>
      </div>
      <button type="submit" className="btn btn--primary" disabled={busy}>
        {t("groups.create")}
      </button>
    </form>
  );
}
