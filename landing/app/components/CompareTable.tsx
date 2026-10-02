import { Check } from "lucide-react";
import CornerTicks from "./CornerTicks";
import { products, type CompareTable as Table } from "../content/compare";

const BRIDGE = 0;

/** A cell that reads as an absence, so it can be dimmed rather than shouted. */
function isAbsence(value: string) {
  return /^(No|Not offered|Not published|Not applicable|Closed source|Limits apply)$/.test(value);
}

/** A cell that reads as a plain yes, so it can carry the mark instead of the word. */
function isAffirmative(value: string) {
  return value === "Yes";
}

function Cell({ value, bridge }: { value: string; bridge: boolean }) {
  if (isAffirmative(value)) {
    return (
      <span className={`inline-flex items-center gap-1.5 ${bridge ? "text-foreground" : "text-body"}`}>
        <Check className="size-3.5 shrink-0" strokeWidth={2.5} aria-hidden="true" />
        Yes
      </span>
    );
  }
  const tone = isAbsence(value) ? "text-faint" : bridge ? "text-foreground" : "text-muted-foreground";
  return <span className={tone}>{value}</span>;
}

export default function CompareTable({ table }: { table: Table }) {
  return (
    <div className="reveal relative mt-10">
      <CornerTicks />
      <div className="overflow-x-auto rounded-xl border border-border-card">
        <table className="w-full min-w-[1040px] border-collapse text-left text-[13px]">
          <caption className="sr-only">{table.title}</caption>
          <thead>
            <tr className="border-b border-border-card">
              <th scope="col" className="w-[300px] px-5 py-4 text-[11px] font-normal uppercase tracking-wider text-muted-foreground">
                Capability
              </th>
              {products.map((product, index) => (
                <th
                  key={product.id}
                  scope="col"
                  className={`px-5 py-4 align-bottom ${index === BRIDGE ? "bg-card" : ""}`}
                >
                  <span className={`block text-[14px] font-semibold ${index === BRIDGE ? "text-foreground" : "text-muted-foreground"}`}>
                    {product.name}
                  </span>
                  <span className="mt-0.5 block whitespace-nowrap font-mono text-[10px] uppercase tracking-[0.12em] text-faint-2">
                    {product.note}
                  </span>
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {table.rows.map((row) => (
              <tr key={row.label} className="border-b border-border last:border-b-0">
                <th scope="row" className="px-5 py-4 align-top font-normal">
                  <span className="block text-[13.5px] font-medium text-foreground">{row.label}</span>
                  {row.hint && <span className="mt-1 block max-w-[280px] text-[12px] leading-5 text-muted-foreground">{row.hint}</span>}
                </th>
                {row.cells.map((cell, index) => (
                  <td key={products[index].id} className={`px-5 py-4 align-top ${index === BRIDGE ? "bg-card" : ""}`}>
                    <Cell value={cell} bridge={index === BRIDGE} />
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
