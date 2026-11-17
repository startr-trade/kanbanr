/**
 * A page section (FTR-046): a `<section>` named by its heading, with actions on the heading's line.
 */
import { useId, type ReactNode } from "react";

export function Section(p: { title: string; level?: 2 | 3; id?: string; actions?: ReactNode; children: ReactNode }): JSX.Element {
  const own = useId();
  const headId = `${p.id ?? own}-title`;
  const Heading = `h${p.level ?? 2}` as "h2" | "h3";
  return (
    <section className="ui-section" id={p.id} aria-labelledby={headId}>
      <div className="ui-section-head">
        <Heading id={headId}>{p.title}</Heading>
        {p.actions && <div className="ui-section-actions">{p.actions}</div>}
      </div>
      {p.children}
    </section>
  );
}
