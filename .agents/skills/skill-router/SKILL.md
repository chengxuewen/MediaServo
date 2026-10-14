---
name: skill-router
description: "Analyze user intent and output a recommended skill list. Auto-activates when the user says '怎么做', '用什么技能' (how to do this / what skill), or intent is ambiguous. Use when the user asks 'what skill should I use', 'how to approach this', or when intent is ambiguous." # c49:trigger
---

# skill-router — Skill Routing Analysis

> When user intent is fuzzy or multiple skills compete, analyze and recommend the best skill combination.

## Trigger Conditions

- User says "怎么做" (how to do this) / "用什么技能" (what skill) / "帮我选择" (help me choose) <!-- c49:trigger -->
- User intent is vague — clarify first, then recommend
- Multiple skills may apply — compare and select

## Analysis Flow

### Step 1: Intent Classification

Read the user message and classify:

| Class | Keywords | Default Recommendation |
|------|--------|---------|
| **Implement** | "添加"/"实现"/"创建"/"修改" (add/implement/create/modify) | `/think-before-act` + `/test-driven-development` | <!-- c49:trigger -->
| **Fix** | "修复"/"bug"/"错误"/"不工作" (fix/bug/error/not working) | `/systematic-debugging` | <!-- c49:trigger -->
| **Refactor** | "重构"/"优化"/"简化"/"清理" (refactor/optimize/simplify/cleanup) | `/think-before-act` + `/remove-ai-slops` | <!-- c49:trigger -->
| **Design** | "设计"/"架构"/"方案"/"怎么做" (design/architecture/plan/how-to) | `/brainstorming` + `/openspec-propose` | <!-- c49:trigger -->
| **Test** | "测试"/"E2E"/"覆盖率" (test/E2E/coverage) | `/test-driven-development` + `/playwright` | <!-- c49:trigger -->
| **Docs** | "文档"/"README"/"说明" (docs/README/explanation) | `/doc-audit` | <!-- c49:trigger -->
| **Security** | "安全"/"权限"/"认证" (security/permissions/auth) | `/security-review` | <!-- c49:trigger -->
| **Explore** | "调研"/"对比"/"有什么方案" (research/compare/what options) | `/ecosystem-scan` + librarian agent | <!-- c49:trigger -->
| **Ambiguous** | no clear keyword | propose 2-3 possible directions, let the user choose |

### Step 2: Context Check

Before recommending, check the current state:

1. **Which tasks are in_progress?** — if a task is already running, only recommend skills relevant to it
2. **Which skills did the user just use?** — avoid re-recommending one that was just used
3. **Current project phase?** — new feature vs fix vs refactor; different phases get different recommendations
4. **Tech-stack match?** — ensure the recommended skills apply to the current stack (Rust/TS/Docker/Web)

### Step 3: Output the Recommendation

**Single-skill case** (clear intent):
```
Recommend: /systematic-debugging
Reason: 2 consecutive fix commits — systematic root-cause diagnosis is needed first
```

**Multi-skill case** (combination needed):
```
Recommended combo:
1. /think-before-act — research options first, avoid blind edits
2. /test-driven-development — write tests before implementing
Reason: non-trivial multi-module change needs a test safety net
```

**Ambiguous case** (needs clarification):
```
Intent unclear, possible directions:
1. Implementing a new feature → /think-before-act + /test-driven-development
2. Fixing a bug → /systematic-debugging
3. Refactoring → /think-before-act
What exactly do you want to do?
```

### Step 4: Execution Suggestion

After recommending, if the user confirms, invoke the skill directly:
```
Confirmed: loading /think-before-act ...
```

## Relationship with context-engineering

- **context-engineering**: passive trigger (auto-recommends when a scenario is detected, via the AGENTS.md directory table)
- **skill-router**: active invocation (deep analysis + recommendation when the user asks "怎么做" or intent is ambiguous) <!-- c49:trigger -->

The two are complementary: context-engineering's routing table handles keyword matching; skill-router handles fuzzy intent and multi-skill combination recommendations.

## When NOT to Recommend

| Scenario | Do not recommend | Reason |
|------|--------|------|
| User is mid-urgent-fix | any skill | don't interrupt, finish the fix first |
| 1-line change | /think-before-act | overkill |
| User explicitly named a skill | other skills | respect the user's choice |
| Same skill just recommended | the same skill | anti-spam |
