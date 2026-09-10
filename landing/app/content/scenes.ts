export type Tone = "success" | "warning" | "info" | "destructive" | "faint";

export type Entry =
  | { kind: "user"; text: string }
  | { kind: "assistant"; text: string }
  | { kind: "rail"; label: string; status?: string; text: string }
  | {
      kind: "notice";
      edge: Tone;
      title: string;
      status?: string;
      text: string;
      caption?: string;
      code?: string;
      actions?: string[];
    }
  | { kind: "collapsed"; label: string };

