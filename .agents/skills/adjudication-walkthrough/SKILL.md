---
name: adjudication-walkthrough
description: "裁决逐项过 -- one-at-a-time adjudication protocol. Use when the user says '裁决逐项过', '一个个来', '不要一起列出', '每项列出细节，方案和对应优缺点、来源、影响、推荐，说人话', or 'walk the options one by one'. Pure text, never batch, seven-part template, plain language."  # c49:trigger
---

# adjudication-walkthrough — One-at-a-Time Adjudication Protocol

> One adjudication per message. Plain text only. Seven-part template. Plain language.

## Trigger Conditions (user phrases -- these are the activation surface)

- Chinese: "裁决逐项过" / "一个个来" / "不要一起列出" / "每项列出细节，方案和对应优缺点、来源、影响、推荐，说人话" (kept verbatim -- they ARE the trigger keywords) <!-- c49:trigger -->
- English: "walk the options one by one", "decide between options", "go through the decision points"
- A plan document contains multiple adjudication points and the user asks to walk them
- Any multi-option design walkthrough where the user asks for one-by-one presentation

## Hard Rules (all from live session lessons -- violating any means rework)

1. **ONE at a time**: one message presents exactly one adjudication. The next one starts only after the user replies ("A" / "pick X" / other opinion). NEVER list several adjudications in one message -- not even two.
2. **Plain text**: the interactive `question` tool is FORBIDDEN for this protocol -- its JSON escaping mangles option formatting (the \r-injection / format-error incident, three times). Present each adjudication as plain text ending with "Reply: Adjudication N -- A / B / C".
3. **Seven-part template**: every adjudication carries all seven sections; missing any means rework:
   - **Details**: what exactly is being decided (2-3 sentences, state the boundary)
   - **Options with pros/cons**: each option gets its own subsection (A/B/C); pros and cons appear in pairs; never hide one side's weakness
   - **Sources**: official docs / repo anchors (file:line) / verified assets -- every claim has provenance
   - **Impact**: what changes for code / docs / users if chosen
   - **Recommendation**: an explicit stand + one-line reason; if there is no room for debate, say "no-contest"
4. **Plain language**: no jargon pileup; every paragraph must survive being read aloud to a colleague outside the CMake/compiler domain. Analogies allowed, jargon stacking forbidden.

## Output Template (follow verbatim)

```
# Adjudication N / total: <title>

## Details
<what is being decided, where the boundary is>

## Options with Pros/Cons

**A. <option name>**
- Pros: ...
- Cons: ...

**B. <option name>**
...

## Sources
- <official docs / repo anchor file:line / verified asset>

## Impact
<concrete changes per party if chosen>

## Recommendation
**<option>**. <one-line reason>.

Reply: Adjudication N -- A / B / C (or state your own take)
```

## Closure Protocol

After all adjudications are walked:

1. **Summary table**: one table with all N adjudications (# / decision / one-line key note);
2. **Write-back**: record the results in the plan document's adjudication section (or a D-entry), dated, marked "user-adjudicated one by one";
3. **Open items**: any adjudication the user skipped or deferred is listed EXPLICITLY -- never silently swallowed.

## Relations to Other Skills

- `think-before-act` governs WHETHER to ask before acting (the decision gate) -- this skill governs the PRESENTATION protocol once asking is warranted. They stack: gate triggers, this protocol presents.
- `lesson-review`: if the user corrects format or method during a walkthrough, file the PIT/rule afterward per lesson-review.
- `skill-router`: routes explicit how-do-I-decide questions here (the Adjudication row).

## Anti-Patterns (forbidden)

| Anti-pattern | Correct behavior |
|---|---|
| Listing 3+ adjudications in one message | One at a time; wait for the reply |
| Presenting adjudications via the question tool | Plain text + "Reply: Adjudication N -- A / B / C" |
| Pros/cons only for the recommended option | Every option gets paired pros AND cons; cons are not hidden |
| Jargon pileup ("semantically isomorphic to") | Plain language: "the name says RUST but serves node -- that is technical debt" |
| Adjudication results not written back | Closure protocol: summary table + plan/D-entry write-back |
