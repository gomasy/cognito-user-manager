import { useEffect, useState } from "react";
import { api } from "../api";
import { errorText, useDateFormat, useNavigate, useT, useToast } from "../hooks";
import type { UserSummary } from "../types";
import { EnabledBadge, StatusBadge } from "./Badge";

function displayName(user: UserSummary): string {
  const composed = [user.attributes.family_name, user.attributes.given_name]
    .filter(Boolean)
    .join(" ");
  return user.attributes.name || composed || "—";
}

interface Props {
  poolName: string;
  /** Served by /api/pool; the server applies the same list to the filter. */
  searchFields: string[];
}

export function AdminUsers({ poolName, searchFields }: Props) {
  const t = useT();
  const navigate = useNavigate();
  const formatDate = useDateFormat();
  const { notify } = useToast();

  const defaultField = searchFields[0] ?? "";
  const [field, setField] = useState(defaultField);
  const [query, setQuery] = useState("");
  const [submitted, setSubmitted] = useState({ q: "", field: defaultField });
  const [token, setToken] = useState<string | undefined>();
  const [users, setUsers] = useState<UserSummary[]>([]);
  const [nextToken, setNextToken] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    api
      .listUsers({ q: submitted.q, field: submitted.field, token })
      .then((page) => {
        if (cancelled) return;
        setUsers(page.users);
        setNextToken(page.nextToken);
      })
      .catch((e) => notify(errorText(e), "error"))
      .finally(() => !cancelled && setLoading(false));
    return () => {
      cancelled = true;
    };
  }, [submitted, token, notify]);

  const search = (event: React.FormEvent) => {
    event.preventDefault();
    setToken(undefined);
    setSubmitted({ q: query, field });
  };

  return (
    <main className="page">
      <header className="page__header page__header--split">
        <div>
          <h1>{t("admin.title")}</h1>
          <p className="hint">
            {t("admin.pool")} <span className="mono">{poolName}</span>
          </p>
        </div>
        <a
          className="btn btn--primary"
          href="/admin/users/new"
          onClick={(event) => {
            event.preventDefault();
            navigate("/admin/users/new");
          }}
        >
          {t("admin.create")}
        </a>
      </header>

      <form className="card row row--gap row--end" onSubmit={search}>
        <label className="field field--tight">
          <span className="field__label">{t("admin.searchField")}</span>
          <select value={field} onChange={(event) => setField(event.target.value)}>
            {searchFields.map((name) => (
              <option key={name} value={name}>
                {t(`search.${name}`)}
              </option>
            ))}
          </select>
        </label>
        <label className="field field--grow">
          <span className="field__label">{t("admin.searchPrefix")}</span>
          <input
            type="search"
            value={query}
            placeholder={t("admin.searchPlaceholder")}
            onChange={(event) => setQuery(event.target.value)}
          />
        </label>
        <button type="submit" className="btn btn--primary">
          {t("admin.search")}
        </button>
        {submitted.q && (
          <button
            type="button"
            className="btn"
            onClick={() => {
              setQuery("");
              setToken(undefined);
              setSubmitted({ q: "", field });
            }}
          >
            {t("admin.clear")}
          </button>
        )}
      </form>

      <div className="table-wrap">
        <table>
          <thead>
            <tr>
              <th>{t("user.username")}</th>
              <th>{t("attr.email")}</th>
              <th>{t("attr.name")}</th>
              <th>{t("user.state")}</th>
              <th>{t("user.status")}</th>
              <th>{t("user.created")}</th>
            </tr>
          </thead>
          <tbody>
            {users.map((user) => (
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
                <td>
                  {user.attributes.email ?? "—"}
                  {user.attributes.email && user.attributes.email_verified !== "true" && (
                    <span className="badge badge--warn">{t("user.unverified")}</span>
                  )}
                </td>
                <td>{displayName(user)}</td>
                <td>
                  <EnabledBadge enabled={user.enabled} />
                </td>
                <td>
                  <StatusBadge status={user.status} />
                </td>
                <td>{formatDate(user.createdAt)}</td>
              </tr>
            ))}
          </tbody>
        </table>
        {!loading && users.length === 0 && (
          <p className="empty">{submitted.q ? t("admin.noMatches") : t("admin.noUsers")}</p>
        )}
        {loading && <p className="empty">{t("common.loading")}</p>}
      </div>

      <div className="row row--between">
        <span className="hint">
          {t("admin.shown", { count: users.length })}
          {nextToken && ` ${t("admin.more")}`}
        </span>
        <span className="row row--gap">
          {token && (
            <button type="button" className="btn" onClick={() => setToken(undefined)}>
              {t("admin.firstPage")}
            </button>
          )}
          {nextToken && (
            <button type="button" className="btn" onClick={() => setToken(nextToken)}>
              {t("admin.nextPage")}
            </button>
          )}
        </span>
      </div>
    </main>
  );
}
