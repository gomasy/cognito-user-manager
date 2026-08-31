import { useCallback, useEffect, useState } from "react";
import { api } from "../api";
import { errorText, useAction, useDateFormat, useNavigate, useT, useToast } from "../hooks";
import type { GroupInfo, UserSummary } from "../types";
import { EnabledBadge, StatusBadge } from "./Badge";
import { Pager } from "./Pager";

interface Props {
  group: string;
  /** The group that grants access to this console, or null if unknown. */
  adminGroup: string | null;
  onGroupsChanged: () => void;
}

export function AdminGroupDetail({ group, adminGroup, onGroupsChanged }: Props) {
  const t = useT();
  const navigate = useNavigate();
  const formatDate = useDateFormat();
  const { notify } = useToast();

  const [info, setInfo] = useState<GroupInfo | null>(null);
  const [missing, setMissing] = useState(false);
  const [members, setMembers] = useState<UserSummary[]>([]);
  const [token, setToken] = useState<string | undefined>();
  const [nextToken, setNextToken] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    api
      .group(group)
      .then(setInfo)
      .catch((e) => {
        setMissing(true);
        notify(errorText(e), "error");
      });
  }, [group, notify]);

  const loadMembers = useCallback(async () => {
    setLoading(true);
    try {
      const page = await api.groupMembers(group, token);
      setMembers(page.users);
      setNextToken(page.nextToken);
    } catch (e) {
      notify(errorText(e), "error");
    } finally {
      setLoading(false);
    }
  }, [group, token, notify]);

  useEffect(() => {
    void loadMembers();
  }, [loadMembers]);

  const { busy, run } = useAction(loadMembers);

  if (missing) {
    return (
      <main className="page page--narrow">
        <div className="card">
          <h2>{t("groups.notFound")}</h2>
          <button type="button" className="btn" onClick={() => navigate("/admin/groups")}>
            {t("groups.backToList")}
          </button>
        </div>
      </main>
    );
  }

  return (
    <main className="page">
      <header className="page__header">
        <p className="hint">
          <a
            href="/admin/groups"
            onClick={(event) => {
              event.preventDefault();
              navigate("/admin/groups");
            }}
          >
            {t("groups.backToList")}
          </a>
        </p>
        <div className="page__header--split">
          <h1 className="mono">{group}</h1>
          {group === adminGroup && <span className="badge badge--ok">{t("nav.admin")}</span>}
        </div>
      </header>

      <div className="card">
        <h2>{t("detail.overview")}</h2>
        <dl className="dl">
          <dt>{t("groups.description")}</dt>
          <dd>{info?.description || "—"}</dd>
          <dt>{t("groups.precedence")}</dt>
          <dd>{info?.precedence ?? "—"}</dd>
          <dt>{t("user.created")}</dt>
          <dd>{formatDate(info?.createdAt ?? null)}</dd>
          <dt>{t("user.updated")}</dt>
          <dd>{formatDate(info?.updatedAt ?? null)}</dd>
        </dl>
      </div>

      <AddMemberCard
        busy={busy}
        // The new member is sorted in wherever Cognito puts them, which is
        // rarely the page that happens to be open.
        onAdd={async (username) => {
          const added = await run(() => api.addGroupMember(group, username), token === undefined);
          if (added) setToken(undefined);
          return added;
        }}
      />

      <div className="card">
        <h2>{t("groups.members")}</h2>
        <div className="table-wrap">
          <table>
            <thead>
              <tr>
                <th>{t("user.username")}</th>
                <th>{t("attr.email")}</th>
                <th>{t("user.state")}</th>
                <th>{t("user.status")}</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {members.map((user) => (
                <tr key={user.username}>
                  <td className="mono">
                    <a
                      href={`/admin/users/${encodeURIComponent(user.username)}`}
                      onClick={(event) => {
                        event.preventDefault();
                        navigate(`/admin/users/${encodeURIComponent(user.username)}`);
                      }}
                    >
                      {user.username}
                    </a>
                  </td>
                  <td>{user.attributes.email ?? "—"}</td>
                  <td>
                    <EnabledBadge enabled={user.enabled} />
                  </td>
                  <td>
                    <StatusBadge status={user.status} />
                  </td>
                  <td>
                    <button
                      type="button"
                      className="btn btn--danger"
                      disabled={busy}
                      onClick={() =>
                        void run(() => api.removeGroupMember(group, user.username))
                      }
                    >
                      {t("groups.removeMember")}
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          {!loading && members.length === 0 && <p className="empty">{t("groups.noMembers")}</p>}
          {loading && <p className="empty">{t("common.loading")}</p>}
        </div>

        <Pager
          count={members.length}
          token={token}
          nextToken={nextToken}
          onPage={setToken}
        />
      </div>

      <DeleteCard
        group={group}
        // Deleting the group that grants admin access would lock every admin
        // out of this console, and the server refuses it too.
        protected={group === adminGroup}
        busy={busy}
        onDelete={async () => {
          if (await run(() => api.deleteGroup(group), false)) {
            onGroupsChanged();
            navigate("/admin/groups");
          }
        }}
      />
    </main>
  );
}

function AddMemberCard({
  busy,
  onAdd,
}: {
  busy: boolean;
  onAdd: (username: string) => Promise<boolean>;
}) {
  const t = useT();
  const [username, setUsername] = useState("");

  return (
    <form
      className="card row row--gap row--end"
      onSubmit={(event) => {
        event.preventDefault();
        void onAdd(username).then((done) => done && setUsername(""));
      }}
    >
      <label className="field field--grow">
        <span className="field__label">{t("groups.addMember")}</span>
        <input
          type="text"
          value={username}
          autoComplete="off"
          placeholder={t("groups.addMemberPlaceholder")}
          required
          onChange={(event) => setUsername(event.target.value)}
        />
      </label>
      <button type="submit" className="btn btn--primary" disabled={busy}>
        {t("groups.add")}
      </button>
    </form>
  );
}

function DeleteCard({
  group,
  protected: isProtected,
  busy,
  onDelete,
}: {
  group: string;
  protected: boolean;
  busy: boolean;
  onDelete: () => void;
}) {
  const t = useT();
  const [confirm, setConfirm] = useState("");
  const [open, setOpen] = useState(false);

  return (
    <div className="card card--danger">
      <h2>{t("detail.danger")}</h2>
      <p className="hint">{t("groups.deleteHint")}</p>
      {isProtected ? (
        <p className="alert alert--warn">{t("groups.adminProtected", { group })}</p>
      ) : open ? (
        <form
          onSubmit={(event) => {
            event.preventDefault();
            onDelete();
          }}
        >
          <label className="field">
            <span className="field__label">{t("groups.confirmDelete", { group })}</span>
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
              disabled={busy || confirm !== group}
            >
              {t("groups.deleteForever")}
            </button>
            <button type="button" className="btn" onClick={() => setOpen(false)}>
              {t("common.cancel")}
            </button>
          </div>
        </form>
      ) : (
        <button type="button" className="btn btn--danger" onClick={() => setOpen(true)}>
          {t("groups.delete")}
        </button>
      )}
    </div>
  );
}
