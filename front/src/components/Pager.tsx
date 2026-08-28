import { useT } from "../hooks";

interface Props {
  /** Rows on the page being shown. */
  count: number;
  /** The token that got here, if this is not the first page. */
  token: string | undefined;
  /** The token for the page after this one, if there is one. */
  nextToken: string | null;
  onPage: (token: string | undefined) => void;
}

/**
 * The footer under both admin lists. Cognito pages by an opaque token that
 * only ever points forward, so there is a first page and a next one and no way
 * to count what lies between them.
 */
export function Pager({ count, token, nextToken, onPage }: Props) {
  const t = useT();

  return (
    <div className="row row--between">
      <span className="hint">
        {t("admin.shown", { count })}
        {nextToken && ` ${t("admin.more")}`}
      </span>
      <span className="row row--gap">
        {token && (
          <button type="button" className="btn" onClick={() => onPage(undefined)}>
            {t("admin.firstPage")}
          </button>
        )}
        {nextToken && (
          <button type="button" className="btn" onClick={() => onPage(nextToken)}>
            {t("admin.nextPage")}
          </button>
        )}
      </span>
    </div>
  );
}
