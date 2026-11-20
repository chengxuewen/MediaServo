// @mediaservo/talk-ui — intercom component package (talk-app T0 scaffold).
//
// Design authority: monitor-center K1–K16 (+17-item backlog, §6d layer order).
// This package renders it; it never re-adjudicates design (J1=A).
//
// Planned components land batch-by-batch per talk-app §4 (scaffold first):
//   T5  board row set       — FloorBoard / FloorRow (mode, hue+shape state glyph,
//                             holder, timer, queue, guarded actions, unknown
//                             state on disconnect)
//   T6  roster              — MemberList (username/role from server roster)
//   T8/T9                   — ReplayBar, EvidencePanel
//   T11 session view        — ChannelSession / PTTButton (hold-to-talk, cues,
//                             Space/pointer discipline, Esc arbitration)
// Everything styles via --ms-* only (@mediaservo/ui TOKENS.md is the public
// contract); behavior lives in @mediaservo/client (BoardStore) — this package
// is React rendering + local view state only (headless-friendly, D4).

export const VERSION = '0.1.0';
