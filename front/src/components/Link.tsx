import type { ReactNode } from "react";
import { useNavigate } from "../hooks";

/** Escaped, so a username holding a slash still resolves. */
export const userRoute = (username: string) =>
  `/admin/users/${encodeURIComponent(username)}`;

export const groupRoute = (group: string) =>
  `/admin/groups/${encodeURIComponent(group)}`;

interface Props {
  to: string;
  className?: string;
  children: ReactNode;
}

/**
 * An in-app link. A plain click is routed without a reload; a modified or
 * middle click is left to the browser, which keeps open-in-new-tab working.
 */
export function Link({ to, className, children }: Props) {
  const navigate = useNavigate();

  return (
    <a
      href={to}
      className={className}
      onClick={(event) => {
        if (
          event.button !== 0 ||
          event.metaKey ||
          event.ctrlKey ||
          event.shiftKey ||
          event.altKey
        ) {
          return;
        }
        event.preventDefault();
        navigate(to);
      }}
    >
      {children}
    </a>
  );
}
