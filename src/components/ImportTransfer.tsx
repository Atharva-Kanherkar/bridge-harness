import type { ReactNode } from "react";
import { Check } from "lucide-react";
import { BridgeGlyph } from "./BridgeGlyph";
import { HarnessMark } from "./harnessMarks";

// The transfer stage on the Import page, after a file copy on macOS or
// Windows: the tool on the left, Bridge on the right, and chats arcing across
// between them while the import runs. Bridge's tile pulses as each one lands.
// When the import ends the cards stop and Bridge shows a check. Under reduced
// motion the cards are not drawn at all; the stage stays as a still picture.

const FLIGHTS = 6;

const TOOLS: Record<string, { label: string; harness: string }> = {
  claude_code: { label: "Claude Code", harness: "claude" },
};

export function ImportTransfer({ tool, done = false }: { tool: string; done?: boolean }) {
  const source = TOOLS[tool] ?? { label: tool, harness: tool };
  return <div className="import-transfer flex items-center gap-3" data-transfer={done ? "done" : "running"}>
    <Tile label={source.label}>
      <HarnessMark harness={source.harness} size={26} />
    </Tile>
    <div className="import-transfer-lane relative h-16 min-w-0 flex-1" aria-hidden="true">
      <svg className="absolute inset-0 h-full w-full overflow-visible" viewBox="0 0 100 64" preserveAspectRatio="none">
        <path className="import-transfer-path" d="M 2 46 Q 50 2 98 46" fill="none" vectorEffect="non-scaling-stroke" />
      </svg>
      {!done && Array.from({ length: FLIGHTS }, (_, index) => <span
        key={index}
        className="import-transfer-flight"
        style={{ animationDelay: `${(index * 2.4) / FLIGHTS}s` }}
      >
        <span className="import-transfer-arc" style={{ animationDelay: `${(index * 2.4) / FLIGHTS}s` }}>
          <span className="import-transfer-card" style={{ animationDelay: `${(index * 2.4) / FLIGHTS}s` }}>
            <i /><i /><i />
          </span>
        </span>
      </span>)}
    </div>
    <Tile label="Bridge" target done={done}>
      <BridgeGlyph size={26} />
    </Tile>
  </div>;
}

function Tile({ label, target = false, done = false, children }: { label: string; target?: boolean; done?: boolean; children: ReactNode }) {
  return <div className="flex w-20 shrink-0 flex-col items-center gap-1.5">
    <div className={target ? "import-transfer-tile import-transfer-target" : "import-transfer-tile"}>
      {children}
      {done && <span className="import-transfer-check" aria-hidden="true"><Check size={11} strokeWidth={3} /></span>}
    </div>
    <span className="max-w-full truncate text-[11px] text-muted-foreground">{label}</span>
  </div>;
}
