# Техническое задание: мультипровайдерная multi-agent система

Статус: черновик 0.1
Целевые backend: Codex, Claude Code, Gemini CLI, Grok CLI/API.

## 1. Цель

Превратить приложение из Codex-only frontend в локальный control plane, который
одновременно запускает, визуализирует и контролирует агентов разных
провайдеров. Агент любого провайдера должен иметь возможность делегировать
задачу агенту другого провайдера через общий broker.

Система должна разделять четыре понятия:

- `Agent` — логический исполнитель и узел дерева UI;
- `ProviderSession` — внешняя сессия конкретного backend;
- `Task` — переносимая единица работы;
- `Run` — отдельная попытка выполнить задачу конкретным агентом.

Падение процесса не уничтожает задачу: её можно повторить или переназначить
другому провайдеру с сохранением истории и результатов.

## 2. Границы первой версии

Первая мультипровайдерная версия должна:

1. одновременно поддерживать Codex и Claude Code в одном workspace;
2. показывать единое дерево, статусы, логи, approvals и результаты;
3. позволять вручную делегировать задачу между провайдерами;
4. возвращать результат непосредственному отправителю задачи;
5. сохранять задачи и попытки после перезапуска приложения;
6. позволять cancel, retry и reassign;
7. не передавать credentials и полный контекст между провайдерами неявно;
8. открывать упомянутые агентами файлы во встроенном read-only viewer и, для
   локального workspace, во внешнем редакторе с переходом к строке.

Gemini и Grok подключаются после проверки архитектуры вторым backend. Grok
может использовать API и локальный controlled tool host, если стабильного CLI
протокола нет.

В MVP не входят автоматический выбор лучшей модели, автономная маршрутизация,
облачный orchestrator и совместное редактирование несколькими пользователями.

## 3. Архитектура

```text
TUI / CLI / VS Code extension / optional web UI
                       │
       versioned control protocol + replay cursor
                       │
        agent-orchestrator (authoritative writer)
          Task · Run · dependency graph · policy
              journal · inbox/outbox · leases
                       │
              provider-adapter boundary
        ┌──────────────┼──────────────┐
    Codex adapter  Claude adapter  Gemini/Grok adapters
        │
  Codex app-server
```

Провайдеры не обращаются друг к другу напрямую. Любая делегация и любое
межагентное сообщение проходят через broker, получают внутренние ID и
сохраняют причинную связь.

`agent-orchestrator` является единственным authoritative deterministic writer
для Task, Run, dependency transitions, journal, inbox/outbox и policy decision.
Только он разблокирует зависимые задачи и фиксирует terminal outcome.
`codex-claude-mode` является тонким operator-клиентом TUI и, на переходном
этапе, read-only/shadow gateway наблюдения за Codex. Он не содержит второй
broker, scheduler или authoritative database. Владение provider process после
PoC переносится в отдельные долгоживущие adapters/sidecars.

### 3.1. Слои протоколов и интеграций

Протоколы применяются по назначению и не подменяют друг друга:

- **Codex app-server** — прямой adapter API Codex: threads/turns/items,
  streaming, resume/fork, status, usage и server-initiated approvals. Парсинг
  вывода интерактивного CLI не используется. Официальное описание:
  <https://github.com/openai/codex/blob/main/codex-rs/app-server/README.md>.
- **ACP (Agent Client Protocol)** — граница editor/IDE ↔ coding agent там, где
  provider её поддерживает. ACP не является внутренним broker или durable
  task store: <https://agentclientprotocol.com/get-started/architecture>.
- **MCP** — tools, RAG/code-intelligence resources и broker-aware skills для
  моделей. MCP не является scheduler. Его JSON-RPC primitives и transports
  описаны в <https://modelcontextprotocol.io/docs/learn/architecture>; Tasks
  extension отслеживается, но не становится источником истины до стабилизации
  (<https://blog.modelcontextprotocol.io/posts/2026-07-28/>).
- **A2A** — внешняя federation/discovery граница для удалённых агентов через
  Agent Card, Task, Message, Artifact, streaming и push. Внутренний journal
  остаётся богаче A2A: <https://a2a-protocol.org/latest/specification/>.
- **AG-UI** — необязательный transport/event mapping для будущего web frontend,
  не provider protocol и не storage contract:
  <https://docs.ag-ui.com/concepts/architecture>.
- **OpenTelemetry/OTLP** — export traces, metrics и logs. Для model/usage полей
  используются GenAI semantic conventions; prompt/code content остаётся opt-in
  и redacted: <https://opentelemetry.io/docs/specs/semconv/registry/attributes/gen-ai/>.
- **LSP, SCIP и tree-sitter** — готовые building blocks для symbols,
  definitions/references и parsing. Они дополняют bounded lexical search и не
  требуют собственного языка запросов:
  <https://microsoft.github.io/language-server-protocol/>,
  <https://github.com/sourcegraph/scip>, <https://tree-sitter.github.io/tree-sitter/>.
- **VS Code/Cursor** — native Explorer, Search, SCM, diff, editor, terminal и
  workspace UX. Наш extension добавляет Agents/Tasks/Attention/Artifacts и
  вызывает нативные API, но не реализует второй IDE.

#### MCP-first SaaS connectors

Jira, Confluence, GitHub, GitLab, Linear, Notion и аналогичные SaaS подключаются
MCP-first. Core не содержит bespoke REST-клиенты этих продуктов. Исключение —
Telegram/Slack notification/interaction adapters из §7.1: они доставляют
attention и ограниченные operator actions, а не являются общим SaaS tool layer.

`McpConnectorRegistry` хранит versioned profiles отдельно для workspace и
provider: server identity/transport, discovered capabilities, разрешённые
tools/resources/prompts, read/mutation scope, timeout и bounds. Profile
ссылается на scoped OAuth/secret refs; orchestrator и journal никогда не хранят
raw access/refresh tokens. Remote MCP переиспользуется через тот же registry и
policy, без отдельного vendor transport в core.

Перед выдачей agent adapter выполняет capability discovery и пересекает
фактические server tools/resources с workspace allowlist, principal/Task scope
и effective permissions. Read и mutation разделены. `create`, `update`,
`comment`, `transition`, delete и изменение sharing требуют typed approval с
server, tool, exact arguments digest, external object refs, expiry и ожидаемой
version. Все вызовы получают audit/correlation/idempotency metadata; неизвестный
tool/schema/version fail closed.

Jira issue и Confluence page отображаются в Task как external context,
Artifact/link, но не становятся canonical Task. Reference сохраняет provider,
tenant/site, immutable external ID, canonical URL, object type, observed version/
updated timestamp, fetched-at и content digest. Повторное mutation проверяет
freshness/version; stale object требует refetch и нового approval. Fetch всегда
bounded по objects/bytes/pages, проходит ACL и redaction. Whole-space indexing
по умолчанию выключен. Внешний текст считается untrusted prompt-injection
content, цитируется как evidence и не может создавать tool/system instructions.

Codex, Claude, Gemini и Grok получают логически эквивалентный MCP profile,
переведённый adapter-ом в vendor configuration format. Adapter сообщает exact
server/tool mapping и `supported | degraded | unsupported`; отсутствие remote
MCP или OAuth flow не маскируется локальным credential, proxy tool либо prompt.
Capability parity проверяется до compare/delegate.

Acceptance требует mock MCP servers для discovery, resources и mutation tools;
golden tests эквивалентной конфигурации всех provider adapters; security tests
OAuth scope/secret leakage, tool allowlist, confused deputy, stale version,
approval digest/replay, tenant/workspace isolation, bounds/redaction и malicious
resource text. Network tests используют mock transport и не требуют SaaS creds.

Локальный MVP использует Unix domain socket и versioned JSONL envelopes с
snapshot, монотонным sequence, replay cursor, idempotency key и bounded payload.
Первый удалённый transport — SSH stdio после отдельного ADR. Network daemon,
WebSocket/HTTPS, A2A и AG-UI не входят в начальный PoC.

### 3.2. Контракт backend-адаптера

Каждый адаптер обязан предоставлять:

- probe backend и его версии;
- декларацию capabilities;
- create/resume/close session;
- start turn/run;
- streaming нормализованных событий;
- send input/follow-up, если поддерживается;
- cancel;
- resolve approval;
- получение effective permissions;
- usage/cost/context, если доступны.

Capabilities включают `resume_session`, `interactive_input`,
`structured_tool_events`, `native_subagents`, `approval_requests`,
`permission_profiles`, `diff_artifacts`, `usage_reporting`, `cost_reporting`,
`session_fork` и `remote_execution`.

UI не имитирует отсутствующую функцию. Команда скрывается либо показывает
явный fallback и его ограничения.

### 3.3. Нормализованные события

Минимальные типы:

- agent/session/run started, status changed, completed, failed, cancelled;
- assistant text delta/completed;
- tool started/progress/completed/failed;
- file change proposed;
- approval requested/resolved;
- user input requested;
- task delegated/reassigned;
- inter-agent message;
- artifact produced;
- usage/context updated;
- backend disconnected/reconnected;
- heartbeat, stalled и diagnostic.

Каждое событие содержит внутренние `workspace_id`, `agent_id`, `task_id`,
`run_id`, sequence, timestamp, causal ID и bounded payload. Неизвестное событие
провайдера сохраняется как ограниченная диагностика, но не разрешает действие.
Reasoning отображается только как текущая фаза без сохранения скрытого CoT.

## 4. Доменная модель

### Agent

Содержит внутренний ID, случайное display name, provider/model, внешний session
ID, parent/root agent, cwd, capabilities, requested/effective permissions,
status, timestamps и путь к provider log. Технические ID не показываются в
основном дереве.

Статусы: `starting`, `idle`, `running`, `waiting_on_agent`, `waiting_on_user`,
`approval_required`, `cancelling`, `completed`, `failed`, `cancelled`,
`disconnected`, `stalled`.

### Task

Содержит инструкции, создателя, владельца и получателя результата, parent и
dependencies, required capabilities, context refs, cwd, permission ceiling,
priority, budget/deadline, status, attempts и итоговый result.

### Run

Одна попытка выполнения Task. Содержит agent/provider request, status,
timestamps, permission snapshot, usage/cost, checkpoint и terminal outcome.
Retry всегда создаёт новый Run.

### Message

Содержит sender, recipient, task/run, kind, content, artifact refs,
correlation/causation IDs, delivery/ack timestamps и idempotency key.

### Artifact

Diff, файл, отчёт, изображение, лог или ссылка. Большие данные хранятся отдельно
и передаются по storage ref + digest.

### Git project forest

Workspace содержит `ProjectForest`, а не предполагает один repository root.
Узел `Repository` имеет стабильную identity, canonical root, VCS kind, remote,
branch/detached HEAD, HEAD SHA, dirty/ahead-behind/conflict state, parent nested
repository или submodule relation. `Worktree` принадлежит Repository и хранит
path/URI, branch/HEAD, owner Agent/Task/Run, lifecycle и cleanup policy.

Один workspace может включать multi-root repositories, nested repositories,
submodules и несколько worktree одного repository. Path всегда разрешается в
конкретные workspace/repository/worktree identities; longest matching root не
может молча пересечь symlink, submodule или nested-repository boundary.
File-change/compare Artifact хранит repository/worktree, `base_sha`,
`subject_sha` либо dirty overlay digest и ownership. Compare строится только при
совместимой базе либо явно показывает non-equivalent base.

### Approval

Содержит источник, task/run/agent, тип опасного действия, summary/details,
requested scope, permission ceiling, choices/default, срок и решение.

### 4.1. Multi-provider session management

Система разделяет пять сущностей: `WorkspaceSession` — долгоживущий контейнер
workspace, policy, задач и истории; `LogicalAgent` — стабильная внутренняя
identity исполнителя; `ProviderSession` — конкретная привязка к provider/model
и внешней сессии; `Run`/`Turn` — попытка Task и отдельный интерактивный обмен;
`ClientConnection` — подключение TUI/CLI/extension/web с user/device identity,
RBAC, cursor, lease и observed revision.

Internal ID стабильны, opaque и не переиспользуются. External provider ID
хранятся отдельно, namespaced по provider/host/account и не используются как
authorization identity. Имя, tags, provider/model, cwd, worktree и owner —
изменяемые metadata. `SessionGroup` служит навигации и bulk actions, но не
повышает permissions. Versioned `SessionTemplate` задаёт provider/model
selectors, cwd/worktree strategy, permission ceiling, tool/network policy и
budgets; созданная сессия хранит snapshot версии шаблона.

Task принадлежит WorkspaceSession и LogicalAgent. ProviderSession исполняет
Run, но не становится владельцем Task. Переназначение Task другому provider
создаёт новый Run/ProviderSession и не является миграцией vendor session: её
скрытая история, credentials и provider-specific state не переносятся.

Lifecycle поддерживает `create`, `list`, `select`, `resume`, `reconnect`,
`suspend`, `close`, `archive`, `unarchive`, `clone`, `fork`, `import`,
`adopt orphan`, `export` и `delete`. Adapter объявляет каждую операцию как
`native`, `emulated` с явными потерями либо `unsupported`; UI не выдаёт clone
за fork или новую сессию за resume. Import/adopt требуют probe, сверки provider
ID, host/account, cwd, protocol nonce, ownership и отсутствия другого owner.

`close` прекращает процессы/leases, сохраняя историю; `archive` скрывает closed
из обычной навигации; `clone` копирует только настройки; `fork` использует
provider-native history только при capability. Delete разрешён для terminal/
closed targets после preview, approval и grace period с учётом retention/legal
hold. Export создаёт versioned manifest с ID mapping, Messages, events,
permission snapshots и Artifact digests без credentials, secrets и host tokens.

Bulk close/archive/tag/export/delete/cancel сначала разрешает фильтр в
revision-bound snapshot и показывает targets, active Runs/Tasks, unread,
artifacts, remote owners и последствия. При изменении revision apply отклоняется
и требует нового preview. Destructive, active, remote и ownership-changing
операции требуют approval, RBAC и audit reason.

Состояния lifecycle и attention независимы: active включает running/waiting/
approval/disconnected с живым lease; closed больше не исполняется; historical
включает closed/archived; unread сохраняется для результата, вопроса или ошибки
после user cursor. Closed скрывается из active picker, но unread closed остаётся
в Attention. Для `(user/client, LogicalAgent)` сохраняются draft, input-history
cursor, log scroll/anchor, selected view, unread cursor и раскрытые группы.
Конфликт draft между clients разрешается явно, без silent last-write-wins.

List/search имеют pagination, стабильную сортировку и фильтры по group/tag/name,
provider/model, cwd/worktree, owner, Task, lifecycle, health, unread,
permissions и времени. Поиск логов/Artifact соблюдает ACL, retention и
redaction. Internal ID используется только как tie-breaker, не как основной UI.

ProviderSession публикует heartbeat и lease. Health отдельно отражает process,
transport, resumability и progress; `disconnected`, `stalled`, `expired` и
`orphaned` не смешиваются. После crash broker reconciles event journal,
materialized state, process table и provider probe. Пока ownership не доказан,
найденный процесс quarantined как orphan; terminal Run не оживает от позднего
event, а unknown external side effect не retry автоматически.

WorkspaceSession имеет один authoritative writer lease и несколько clients.
Mutations несут `expected_revision` и idempotency key. Approval, Turn, close,
delete и reassign используют compare-and-swap: stale client получает conflict,
актуальный snapshot/diff и должен явно повторить намерение. Потерявший leader
lease прекращает mutations до election/reconciliation.

Permissions представлены requested settings, backend-confirmed effective
settings и immutable Run/Turn snapshots. Resume/reconnect/import/adopt повторно
проверяют policy и не восстанавливают прежний более высокий scope. Remote
sessions имеют tenant/project/workspace owner, host identity и отдельные RBAC
права view/interact/approve/manage/export/delete; все lifecycle, ownership,
permission, bulk, export и delete действия аудитируются.

Logs, events, Messages и Artifact связаны стабильными session/task/run ID и
digest. Retention раздельна для metadata, logs, raw provider logs и blobs.
Legal hold и pinned/shared Artifact блокируют GC; reference-aware mark/GC не
удаляет данные, используемые другим Run. Preview удаления показывает retained
и cascading data, а archive не разрывает ссылки.

```text
/sessions [--active|--history|--unread] [filters]
/session new|select|resume|reconnect|suspend|close|archive|unarchive
/session clone|fork|import|adopt|export|delete
/session group|tag|rename|template
/session bulk <action> <filter>
```

Меню показывает capabilities/unsupported reason, owner, health, activity,
unread, cwd/worktree, provider/model и effective permissions. Delete не имеет
одноклавишного shortcut; destructive и bulk команды всегда открывают preview.

## 5. Broker и делегирование

Быстрые команды:

```text
/delegate claude review this patch
/delegate @Turing investigate failing test
/send @Curie check approval handling
/retry task-17 gemini
/reassign task-17 codex
/cancel task-17
```

Порядок делегирования:

1. создаётся Task с bounded context;
2. выбирается существующий idle agent либо создаётся новый;
3. проверяются capabilities, policy, concurrency и budget;
4. создаётся Run и фиксируется permission snapshot;
5. результат возвращается создателю как Message + Artifact refs;
6. если родитель завершён, результат остаётся непрочитанным для пользователя.

Ограничения:

- максимальная глубина по умолчанию 4;
- ограничиваемый fan-out;
- запрет делегирования предку и циклов dependencies;
- не более одного активного Run на Task, кроме режима compare;
- доставка сообщений at-least-once с дедупликацией;
- mutating commands имеют idempotency key;
- автоматический retry только для классифицированных transient failures;
- неизвестный результат external side effect не повторяется автоматически.

### 5.1. Structured `@` references и маршрутизация результатов

При вводе `@` composer открывает fuzzy autocomplete доступных сущностей:
агентов, `provider:model`, task-role и channel. Канонический синтаксис:

```text
@agent:Ada
@codex:gpt-5.4
@role:research
@channel:research
```

Таким образом provider/model использует форму `@provider:model`. Поиск допускает
короткий ввод (`@Ada`, `@codex`) и показывает тип, display name,
provider/model, status и scope. `↑/↓` или `Ctrl-N/Ctrl-P` выбирают вариант,
`Tab`/`Enter` вставляют semantic chip, `Esc` закрывает список. Несколько
получателей разрешены. На экране chip выглядит компактно, например
`[Codex][Ada]`, но хранит typed reference `{kind, stable_id, display_snapshot}`;
переименование не меняет адресата. Backspace рядом с chip удаляет его атомарно.
На узком терминале chip сокращается до `[Ada]`, оставаясь раскрываемым.

Copy/paste сериализует chip в канонический текстовый `@kind:value`; вставленный
текст повторно разрешается через registry. Неоднозначная или устаревшая ссылка
не отправляется молча: composer показывает варианты либо требует замены.
Произвольный текст, похожий на `@`, можно экранировать.

Execution target и `result_recipient` — разные поля Task. Фраза пользователя
`сделай X, результат отправь в @...` остаётся читаемым prompt, однако выбор chip
создаёт structured routing metadata; broker не полагается на LLM/NLP parsing.
Перед отправкой preview явно показывает исполнителя и получателей результата.
Если chip вставлен без выбранного routing slot, UI предлагает `execute by`,
`send result to`, `subscribe` или `mention`; неоднозначное назначение запрещает
отправку. Один Task может иметь несколько result recipients с отдельными
delivery/ack состояниями.

Agent или role можно настроить как collector/subscriber для topic/tag, например
`research`: подходящие результаты доставляются в его Inbox через broker rule.
Channel является адресуемым topic с явным membership и policy, а не агентом.
Inbox показывает unread, sender, Task/Run, topic, delivery time и ack; повторная
доставка дедуплицируется по message/idempotency key.

Если recipient offline, сообщение остаётся durable pending. Если session
закрыта, policy выбирается явно: доставить логическому Agent после resume,
наследнику role/channel, fallback recipient или вернуть sender typed
`recipient_closed`; автоматическая переадресация без отображаемого правила
запрещена. Внешний или cross-workspace recipient требует capability и approval.
Все маршруты пересекаются с workspace membership, task visibility, permission
ceiling и secret policy; знание stable ID само по себе не даёт доступа.

## 6. Permissions и безопасность

Общие профили: `read_only`, `workspace_write`, `full_access`, `custom`.

Effective permissions вычисляются как пересечение workspace policy, permission
ceiling задачи, профиля агента и возможностей backend. Дочерний агент не может
получить больше полномочий, чем родительский Run. Межпровайдерное переназначение
не повышает полномочия.

UI показывает requested и effective permissions; `full_access` выделяется
красным. Изменение профиля существующего агента должно подтверждаться backend,
а не только локальным состоянием UI. Активный Run сохраняет snapshot, новое
значение применяется к последующим действиям.

Credentials не хранятся в общей БД и не копируются между backend. Секреты
передаются только opaque references. Cross-provider secret access по умолчанию
запрещён. Агент не может подтвердить собственный запрос полномочий.

### 6.1. Provider-neutral isolation policy

Vendor-сущности (`sandbox`, permission mode, approval mode, tool allowlist,
container, workspace trust) не являются общей моделью безопасности и не
сравниваются по имени. Для каждого Agent и Run система хранит раздельно:

- `requested_policy` — намерение пользователя или родителя;
- `permission_ceiling` — максимальные полномочия, разрешённые цепочкой
  workspace → Task → parent Run;
- `provider_capabilities` — что backend утверждает, что умеет ограничивать;
- `enforcement_plan` — выбранные независимые уровни изоляции и их параметры;
- `effective_enforcement` — реально применённые ограничения после запуска;
- `evidence` — проверяемые сведения: provider acknowledgement, process/container
  identity, namespace/cgroup IDs, mounts, policy digest и результаты probes.

Запуск разрешается только если effective policy не шире пересечения
`requested_policy ∩ permission_ceiling`. Неподдержанная обязательная грань
останавливает Run до выбора пользователя. Запрещены silent downgrade и
подмена отсутствующей изоляции частыми approvals. Если пользователь явно
разрешил запуск без требуемой sandbox, UI постоянно показывает красный статус
`UNSANDBOXED`, причину, отсутствующие гарантии и scope принятого исключения.

Policy является многомерной, а не одним enum:

- filesystem: разрешённые read/write/create/delete roots, mounts и temporary
  storage;
- network: deny, allowlist host/port/protocol, local-only либо unrestricted;
- process: spawn/exec allowlist, descendants, ptrace, signals и process count;
- environment: разрешённые переменные и принудительно удаляемые значения;
- secrets: доступные opaque secret refs, способ injection и срок жизни;
- resources: CPU, memory, process/file descriptors, disk/output, runtime и
  network quotas;
- devices/IPC: device nodes, Unix sockets, named pipes, shared memory и host
  services.

Предопределённые профили:

- `read_only`: workspace read, отдельный writable temp, сеть и secrets закрыты;
- `workspace_write`: запись только в workspace/worktree и temp, сеть закрыта по
  умолчанию;
- `network_limited`: базовый файловый профиль плюс явный network allowlist;
- `full_access`: ограничения host user, но без обещания sandbox; всегда красный;
- `custom`: явные значения всех измерений и обязательный preview diff policy.

Профиль — только шаблон policy. В журнале и UI отображаются результирующие
измерения, а не только его название. Изменение policy не меняет уже активный
Run: создаётся новый immutable snapshot либо Run перезапускается после
подтверждения. Child Task/Run не может расширить ceiling родителя даже при
переходе к другому provider или isolation backend.

### 6.2. Isolation backends и композиция защиты

Orchestrator выбирает максимально сильную доступную композицию уровней:

1. **Provider-native.** Используются native sandbox, tool policy и approval
   settings Codex/Claude/Gemini. Они считаются дополнительным уровнем, пока
   adapter не предоставил capability и evidence их фактического применения.
2. **Host process.** Отдельные cwd/worktree и OS identity/Unix permissions,
   очищенное environment, ограниченные inherited descriptors и process group.
   Один cwd или отдельный git worktree не является security boundary.
3. **Linux OS sandbox.** User/mount/PID/network namespaces через `bwrap` или
   эквивалент, read-only bind mounts, minimal `/proc` и `/dev`, seccomp; при
   доступности дополнительно Landlock, AppArmor или SELinux. Отсутствующие
   механизмы отражаются в effective evidence.
4. **Resources.** cgroups v2 ограничивает CPU, memory, pids и I/O и поставляет
   метрики. cgroup сам по себе явно не считается security boundary.
5. **Rootless container.** Docker/Podman с pinned image digest, без privileged,
   host PID/network/device mounts, с dropped capabilities, `no-new-privileges`,
   read-only rootfs и явными volume/network rules.
6. **VM/remote executor.** Используется для сильной изоляции недоверенного кода;
   host identity, image/template digest, attestation/capabilities и cleanup
   входят в evidence.

На macOS применяется доступная provider-native изоляция, отдельный user/process
контекст и platform sandbox/container/VM backend. Linux-механизмы нельзя
обещать на macOS; Docker Desktop изолирует VM, но host mounts сохраняют свои
риски. Любая зависимость от устаревших или недоступных sandbox-профилей должна
быть version-gated и явно показана.

На Windows применяются Job Objects для lifecycle/quotas, restricted token и
AppContainer там, где backend совместим. WSL считается отдельным Linux host с
явно объявленными Windows mounts и bridge/network boundary; Job Objects и WSL
сами по себе не дают полной filesystem isolation. Для сильной границы
используется Windows Sandbox/VM или remote executor.

### 6.3. Filesystem, IPC и supply-chain требования

- Все roots канонизируются до запуска; policy хранит logical и resolved path.
- Symlink traversal, hardlink и mount replacement проверяются при открытии,
  записи и передаче Artifact, а не только при первоначальном path check.
- Запрещены неявные host mounts, `/`, home, credential directories, container
  engine sockets, SSH/GPG agents, Docker/Podman socket и произвольные Unix
  sockets.
- `/proc`, `/sys`, devices и IPC доступны только минимально необходимым
  read-only набором; raw devices и privileged mounts запрещены.
- Workspace write рекомендуется выполнять в отдельном worktree/overlay;
  публикация изменений в основной checkout является отдельным действием.
- Container/VM images задаются immutable digest, source/provenance, build time и
  vulnerability/policy status. Mutable tag без resolved digest блокирует
  доверенный профиль.
- Secrets передаются как short-lived file/fd/agent reference, не через общий
  environment или prompt; они редактируются из logs и artifacts.

### 6.4. Capability negotiation по провайдерам

Adapter преобразует vendor settings в нормализованные capability claims и
после запуска возвращает effective evidence:

- **Codex:** sandbox/approval/permissions app-server, фактический ответ
  `turn/start`, версия протокола и provider process boundary;
- **Claude Code:** permission mode, allowed/denied tools и sandbox/hook settings
  конкретной версии; hooks не считаются OS sandbox и не являются достаточным
  enforcement для обязательной filesystem/network policy;
- **Gemini CLI:** sandbox/tool settings, extensions и approval behavior
  конкретной версии; отсутствие проверяемого ограничения компенсируется host
  sandbox либо приводит к отказу запуска;
- **Grok CLI/API:** API не имеет native host sandbox. Все shell/file/network
  tools предоставляет controlled local tool host внутри выбранного isolation
  backend; remote model endpoint учитывается отдельно в network policy.

Capability claim включает `supported`, `unsupported` или `unknown`, semantic
version/range, strength (`advisory`, `provider_enforced`, `os_enforced`,
`vm_enforced`) и evidence source. `unknown` никогда не удовлетворяет
обязательному требованию policy.

### 6.5. Approvals, lifecycle и recovery

Approval описывает точное действие, resolved resources, исходный и целевой
policy snapshot, срок и Run. Одобрение единичного действия не повышает профиль
и не наследуется детьми. Расширение mount/network/secret scope требует нового
Run или атомарного backend reconfiguration с подтверждённым evidence.

Supervisor владеет process tree, namespaces/container/VM и cgroup. При cancel,
crash и restart он обнаруживает и останавливает orphan descendants, отзывает
secrets, unmounts overlays, удаляет temporary worktree/container и сохраняет
необходимые audit evidence. Cleanup идемпотентен; небезопасно определить
владельца ресурса — значит пометить его orphaned и запросить решение, а не
удалять по совпадению имени.

Resource metrics включают CPU time, peak/current memory, pids, disk/I/O,
network bytes, wall time и output volume с отметкой источника и точности.
Превышение hard quota завершает Run typed outcome `resource_limit`; soft limit
создаёт attention event. Метрики и enforcement snapshot сохраняются вместе с
Run для аудита и compare fairness.

### 6.6. Intelligent security advisor и детерминированное решение

Интеллектуальный security analyzer оценивает намерение, команды, scripts, diff,
tool arguments и data flow и предлагает risk, least-privilege policy и понятное
объяснение. LLM reviewer является только advisory layer: он не является
единственным authority/enforcement, не может самостоятельно разрешить действие
и не может одобрить запрос своего Agent/Run.

Решение принимает versioned deterministic policy engine над структурированным
`ExecutionPlan`. До запуска план по возможности содержит разобранные shell,
PowerShell, Python и другие interpreter fragments, resolved paths/URLs,
environment/secret refs, subprocesses, mounts и effective sandbox evidence.
Policy bundle имеет version, digest и подпись; решение действительно только для
точного plan/policy/evidence snapshot.

Проверка выполняется слоями:

1. static parsing/AST, expansion analysis и deterministic allow/deny rules;
2. provenance, content/dependency digests и trust metadata;
3. simulation/dry-run в изоляции, если инструмент предоставляет достоверный
   режим без side effects;
4. LLM review с bounded/redacted input, versioned output schema, citations к
   evidence, confidence и явными unknown;
5. human approval для ambiguous, unknown или high-risk действий;
6. runtime OS/provider enforcement независимо от результата анализа;
7. post-action audit, reconciliation ожидаемых/фактических effects и anomaly
   detection.

Risk score является сигналом и никогда сам по себе не разрешает действие.
Auto-approval допускается только если eligibility matrix детерминированно
подтверждает одновременно: действие low-risk, полностью разобрано, все targets
resolved и digest-bound, оно находится внутри ceiling/effective sandbox,
не использует secrets или запрещённую сеть, не имеет destructive/irreversible
effects и policy прямо разрешает этот action class. Любое `unknown`, parser
fallback, изменившийся digest или более слабый enforcement исключает
auto-approval. Для critical remote actions policy может требовать separation of
duties/two-person approval; инициатор, reviewer и approver фиксируются отдельно.

Analyzer обязан распознавать и повышать risk для:

- prompt injection в script comments, repository content и tool output;
- `eval`, nested interpreters, generated commands, indirect build/package tools;
- obfuscation, base64/decode, download-and-execute и self-modifying scripts;
- destructive glob/path expansion, recursive delete, overwrite и migration;
- network/data exfiltration, secret expansion и unexpected redirects/pipes;
- transitive scripts, imports/includes, package hooks и CI/deploy indirection;
- TOCTOU между review и execution.

Защита от TOCTOU привязывает approval к digest script, parsed plan, referenced
configs/dependencies и resolved targets. Runtime launcher повторно проверяет
digest непосредственно перед execution; изменение требует нового анализа и
approval. Если полное разрешение nested/dynamic behavior невозможно, статус
остаётся `unknown`, а runtime sandbox должен ограничить потенциальное
воздействие.

Каждое решение сохраняется как explainable `SecurityDecisionRecord`: входные
digests, policy/version/signature, deterministic findings, LLM model/version и
bounded findings, citations/confidence, unknowns, requested/effective policy,
approvers, итог, expiry и фактические post-action effects. LLM reasoning/CoT не
сохраняется и не требуется для объяснимости.

### 6.7. Script/Impact Context Graph

Analyzer строит versioned `ScriptImpactContextGraph`, отвечающий не только
«что выполняется», но и «к чему относится» и «что затрагивает». Типы узлов:

- workspace, repository, worktree, module, package, service и component;
- environment: local/dev/test/staging/prod и unknown;
- purpose: build/test/CI/deploy/migration/backup/admin и unknown;
- script/binary/interpreter, import/include и generated/copied source;
- manifest, package script, Make/just target и CI workflow/job/step;
- file/directory, database, cloud account/resource, Kubernetes
  cluster/namespace/object, host, URL/API и downstream service;
- owner/team, secret reference и credential scope.

Рёбра описывают `belongs_to`, `invokes`, `imports`, `includes`, `generated_by`,
`reads`, `writes`, `deletes`, `deploys_to`, `migrates`, `backs_up`,
`authenticates_with`, `calls` и `depends_on`. Детерминированные evidence sources
включают cwd/repo/worktree identity, resolved filesystem links, shebang/AST,
manifests, package scripts, Makefile/justfile, CI definitions, configuration,
command arguments и declared infrastructure metadata.

LLM может добавить semantic labels и предполагаемые связи только с citation,
confidence и происхождением `inferred`; они не подменяют deterministic evidence.
Конфликтующие, неоднозначные или отсутствующие сведения остаются `unknown` и
эскалируются. Название файла, комментарий или утверждение самого script не
являются достаточным доказательством environment/owner/purpose.

Graph обязан учитывать cwd-dependent behavior, symlink и выбранный worktree,
copied/generated scripts, transitive/nested execution и конфигурацию, которая
меняет target во время запуска. Decision привязывается к digest самого script,
всех разрешённых imports/includes, manifests, package/CI definitions и policy-
релевантных configs. Dynamic dependency без стабильного digest делает граф
неполным и запрещает deterministic auto-approval.

UI показывает краткое доказуемое объяснение, например: «относится к service
`billing` в staging; запускает migration; затрагивает database `billing-stg` и
downstream `ledger`; evidence: cwd, Cargo workspace, CI job и resolved URL».
Каждый вывод раскрывается до источника, digest и confidence; inferred и unknown
визуально отличаются от verified.

## 7. UX и мониторинг

Основной экран:

```text
┌ Agents / Tasks ───────┬ Selected agent ──────────────────────────┐
│ Main [Codex]       ●  │ Task, provider/model, parent, permissions │
│ ├─ Ada [Claude]    ◐  ├ Timeline / Log ──────────────────────────┤
│ │  └─ Turing [Gem] ●  │ events, tools, messages, delegation      │
│ ├─ Curie [Grok]    !  │                                          │
│ └─ Ohm [Codex]     ✓  │                                          │
├───────────────────────┴──────────────────────────────────────────┤
│ Composer / active approval                                       │
├──────────────────────────────────────────────────────────────────┤
│ mode · provider · status · permissions · tokens/cost · alerts    │
└──────────────────────────────────────────────────────────────────┘
```

На узком терминале дерево заменяется текущей горизонтальной строкой. Для
каждого агента отдельно сохраняются scroll, draft, unread state и выбранное
представление.

Показываются provider/model, задача, parent/result recipient, status, elapsed,
last activity, effective permissions, tokens/context/cost и attention badge.
Закрытые прочитанные агенты скрыты; завершённый непрочитанный остаётся видимым.

Представления MVP:

- Tree + лог выбранного агента;
- Tasks по состояниям;
- Dashboard по provider/status/attention/usage;
- общая очередь approvals.

Позже: workspace Timeline, dependency graph и Split/Compare.

### Approvals

Approval фонового агента получает визуальный приоритет, но не уничтожает
composer, info overlay, completion или picker: прежнее состояние временно
скрывается и восстанавливается. Несколько запросов образуют очередь Attention
`1/N`. Выбранное решение подсвечено; Enter подтверждает, arrows/j/k меняют,
shortcuts остаются.

### Логи

Режимы `Conversation` и `Timeline`. Thinking показывается только в статусе.
Большие diff открываются в отдельном pager и не вытесняют вопрос подтверждения.
Прокрутка независима для агентов, не прыгает вниз при чтении истории и
показывает счётчик новых событий.

### Сведения

Окно `i` показывает provider session/run/task IDs, cwd, executable, CLI version,
log path и capabilities. Устаревшая CLI выделяется предупреждением; обновление
через `U` требует подтверждения.

### 7.1. Optional Telegram и Slack adapters

Telegram и Slack подключаются только как notification/interaction adapters к
authoritative `agent-orchestrator`. Они не хранят canonical state, не запускают
provider напрямую и не входят в core MVP до стабильных local read bridge,
approval lifecycle и replay cursor. Capability flags отдельно объявляют
`notify`, `status`, `result_delivery`, `approve`, `task_create`, `message`,
`delegate` и `cancel`; неподдерживаемое действие скрывается или отвечает typed
degraded/unsupported, но не эмулируется свободным текстом.

Use cases: status digest, stalled/error/budget alerts, completion и unread
results, approval request/decision, создание bounded Task, сообщение агенту,
явное delegate и cancel. Разрешён только versioned command allowlist с typed
arguments. Ни сообщение, ни callback не могут содержать произвольный shell,
запрос чтения файла или команду выгрузки workspace; file/diff Artifact наружу
передаётся только как redacted summary или авторизованный deep link.

Telegram adapter использует Bot API через webhook либо long polling для
локального MVP. Slack adapter использует Slack App Events API, Socket Mode,
interactivity и allowlisted slash commands. Telegram chat/topic и Slack
team/channel/thread связываются с workspace/Task/Run как routing metadata, но
external thread ID не становится внутренней identity. Кнопки показывают
доступные решения approval с безопасным default, сроком и digest; deep links
открывают соответствующий объект в TUI/IDE/web, не содержат credential или raw
команду.

Identity mapping связывает Telegram `(bot, user, chat)` и Slack `(team, user,
channel)` с orchestrator principal через явный short-lived pairing. Mapping
имеет workspace allowlist, RBAC/scopes, срок, revocation и audit. Group/public
channel по умолчанию получает только redacted notifications; create/delegate/
cancel/approve запрещены до отдельной channel policy. Решение сохраняет
реального external user principal: bot/chat/channel не считаются approver.

Telegram webhook secret/token и Slack signing secret/app/bot tokens поступают
как secret refs или environment injection, никогда не записываются в spec,
workspace config, event journal, logs или error text. Webhook проверяет secret;
Slack проверяет signature и timestamp. Все callbacks защищены expiry, nonce/
request ID, replay window и idempotency key. Approval привязан к точному action,
parameters/policy/subject digest и revision. Expired, revoked, stale или уже
решённый approval отклоняется; при гонке TUI/Slack/Telegram побеждает первая
authoritative compare-and-swap запись, остальные клиенты получают conflict и
фактическое решение.

Messages проходят bounds, escaping и platform formatting без интерпретации
Markdown/mentions как команд. Adapter учитывает size/rate limits, разбивает или
суммирует payload, batch-ит bursts, дедуплицирует notifications и использует
durable outbox/retry с bounded exponential backoff. Delivery и user ack — разные
состояния. Quiet hours подавляют обычные уведомления, но не теряют их; policy
отдельно определяет critical approval/security alerts. Privacy/redaction policy
фильтрует prompts, paths, diffs, secrets, provider logs и user data до отправки.

На Linux и macOS adapter работает как отдельный daemon/sidecar client и не
зависит от открытого TUI. Локально предпочтительны Telegram long polling и Slack
Socket Mode без inbound public endpoint. Webhook/public Events endpoint — это
отдельный remote gateway trust boundary и требует Remote ADR, TLS, host/service
identity, ingress limits и audit; его нельзя неявно включить настройкой bot.

Optional adapter считается принятым после mock Bot API/Slack API tests для
format/escaping, pagination, rate limit/retry, dedupe и ack; security tests для
signature/secret, replay, expiry, revocation, RBAC, public-channel redaction и
двухклиентской approval race; integration tests для offline recovery, stale
digest, task/message/delegate/cancel allowlist и degraded capabilities. Тесты не
обращаются к реальным чатам и не требуют настоящих tokens.

## 8. Минимальный TUI viewer и IDE integration

Продукт не строит полноценные Explorer, Search, SCM, Git graph, language
services, editor или terminal внутри TUI. TUI даёт минимальный терминальный
fallback для навигации, чтения и approval. Основной богатый файловый и Git UX
предоставляет optional VS Code/Cursor extension через нативные Explorer,
Search, SCM, editor/diff, terminal и Remote SSH/Dev Containers/WSL API.

Путь к файлу является навигационной сущностью UI. Ссылки вида `path`,
`path:line` и `path:line:column` распознаются в логах, tool events, diff,
approval details, сообщениях и Artifact. Относительный путь разрешается
относительно `cwd` того Run, который создал ссылку, а не относительно текущего
процесса TUI. Перед открытием нормализованный абсолютный путь и принадлежность
workspace показываются в заголовке viewer.

### 8.1. Встроенный viewer

Встроенный viewer работает без внешних программ и по умолчанию только для
чтения. Это минимальный fallback, не IDE и не замена native SCM/Search/language
services. Он показывает номер строки, базовый syntax highlighting, текущий
`file:line:column`, encoding, размер и признак modified/deleted. Язык
определяется по расширению, shebang и при необходимости ограниченному анализу
содержимого; неизвестный формат показывается как plain text.

Открытие доступно из:

- клика или Enter на file reference в логе/timeline;
- строки файла и hunk header в diff;
- списка путей в file-change approval;
- Artifact типа file/report/log;
- команды `/open <path[:line[:column]]>`.

Клавиши viewer:

- `j/k`, arrows, PageUp/PageDown, Home/End — прокрутка;
- `g` — перейти к строке, `:` — открыть строку команд viewer;
- `/` — поиск, `n/N` — следующее/предыдущее совпадение;
- `Enter` — перейти по выбранной внутренней file reference;
- `o` — открыть текущую позицию во внешнем редакторе;
- `O` — выбрать редактор перед открытием;
- `d` — открыть связанный diff, если он существует;
- `y` — скопировать `path:line`, если clipboard доступен;
- `Esc` или `q` — вернуться точно в прежний лог/diff и scroll position.

Mouse wheel прокручивает содержимое, клик устанавливает текущую строку, клик
по file reference открывает её. Viewer сохраняет отдельную позицию для каждого
файла в пределах workspace. При изменении файла на диске UI предлагает reload;
перезагрузка не происходит молча, если пользователь читает старую версию
артефакта или diff.

Syntax highlighting обязан иметь bounded стоимость: файл читается потоково или
с ограничением размера, подсветка выполняется только для видимого окна с
небольшим запасом. При ошибке определения языка или подсветки viewer остаётся
работоспособным как plain text.

### 8.2. Большие, бинарные и отсутствующие файлы

- Для файла больше настраиваемого лимита (по умолчанию 10 MiB) сначала
  показываются metadata и предупреждение; пользователь может загрузить
  ограниченный диапазон или явно открыть файл внешним редактором.
- Файлы с NUL/признаками binary не декодируются как текст. Показываются тип,
  размер, digest и, если поддерживается, bounded hex/metadata preview.
- Для invalid UTF-8 viewer предлагает lossless hex preview; replacement
  characters не должны незаметно подменять исходное содержимое.
- Для deleted/missing path используется сохранённый Artifact snapshot, если он
  есть; иначе показывается диагностическое сообщение без пустого viewer.
- Symlink отображается как symlink с target. Переход за пределы разрешённых
  корней требует отдельного разрешения и не выполняется автоматически.

### 8.3. Внешний редактор

Настройка редактора имеет следующий приоритет:

1. workspace config `editor.command` и `editor.args`;
2. user config;
3. обнаруженный `code`, затем `cursor`;
4. `VISUAL`/`EDITOR` только после безопасного разбора в argv;
5. отсутствие интеграции с понятной подсказкой настройки.

Поддерживаются явные presets `vscode` и `cursor`. Для них переход выполняется
эквивалентом `--goto <path>:<line>:<column>` и при необходимости `--reuse-window`.
Пользователь может задать argv-template с отдельными placeholders `{file}`,
`{line}`, `{column}`, `{workspace}`. Команда и аргументы хранятся как массив,
не как shell-строка.

Запуск всегда выполняется прямым process spawn без shell, `eval`, конкатенации
или интерпретации metacharacters из пути. Каждый аргумент передаётся отдельно;
пути с пробелами, кавычками, `$`, backticks и Unicode не меняют структуру
команды. Executable разрешается по конфигурации/PATH и проверяется до запуска.
Внешний редактор запускается отсоединённо, его stdout/stderr не повреждают TUI;
ошибка старта возвращается как bounded notification.

Команды:

```text
/open src/main.rs:42
/edit src/main.rs:42:7
/editor
/editor vscode
/editor cursor
```

`/open` всегда открывает встроенный viewer. `/edit` вызывает настроенный внешний
редактор. `/editor` показывает detected executable, источник настройки,
эффективный argv preview и позволяет выбрать preset. В окне `i` workspace
показывается статус editor integration.

### 8.4. Local/remote workspace и permissions

Для локального workspace внешний редактор получает локальный абсолютный путь.
Для remote workspace adapter обязан объявить способ навигации:

- URI, который понимает локальный редактор (например, поддерживаемая remote
  extension);
- безопасное отображение remote path в уже смонтированный local root;
- либо capability `external_editor = unavailable`.

Remote-файл нельзя молча скачивать во временный локальный файл и выдавать его
за рабочую копию. Если доступен только snapshot, он открывается во встроенном
viewer с явной меткой `snapshot/read-only`. Для неподдерживаемого remote path
`/edit` объясняет ограничение и предлагает copy/export только отдельным явным
действием.

Встроенный viewer требует разрешения read для целевого пути. Внешнее открытие
также разрешается только после успешной проверки read и принадлежности
разрешённому local root. Сам факт запуска редактора не выдаёт агенту write
permissions. Если effective profile `read_only`, viewer и заголовок внешнего
редактора помечаются `read-only`; приложение не обещает запретить запись
сторонней программе, но любые последующие изменения остаются внешними и не
считаются одобренным file change агента.

Открытие файла вне workspace, из недоверенного symlink target либо через
remote mapping требует отдельного approval с точным resolved path. Запрос
внешнего редактора не может перекрыть более приоритетный agent approval.

### 8.5. Optional VS Code/Cursor extension

Помимо запуска `--goto` допускается официальное optional extension для VS Code
и Cursor. Оно является тонким клиентом локального orchestrator daemon и не
запускает provider CLI самостоятельно. TUI, extension и будущие GUI используют
один versioned workspace API и видят одинаковые Task, Agent, Run, Approval,
Message и Artifact.

Extension добавляет Activity Bar container со следующими views:

- `Agents` — дерево provider/model, parent/children, status, attention и unread;
- `Tasks` — queued/running/waiting/completed/failed и dependencies;
- `Attention` — approvals и вопросы пользователю;
- `Artifacts` — изменённые файлы, diff, отчёты, изображения и логи;
- `Usage` — tokens, context, cost и budget по workspace/provider/task.

Custom views не дублируют Explorer, Search, Source Control, Testing, Problems,
Outline, editor, diff или terminal. Файлы раскрываются в native Explorer,
repository/worktree — в native SCM, поиск — в Search, diagnostics/tests — через
language/testing API. Git decorations (branch, dirty, ahead/behind, conflicts),
Agent/Task/Run owner и worktree overlay добавляются decorations, badges,
CodeLens, hover и context menus поверх нативных элементов.

Status Bar показывает выбранный workspace, число running/attention, выбранного
агента и effective permissions. `full_access` обозначается текстом и красным
warning color. Клик открывает соответствующий view; входящий approval создаёт
badge и notification, но не забирает фокус редактора автоматически.

Команды extension:

- `Open Multi-Agent Workspace` и `Focus Agent/Task`;
- `Open Log/Timeline`, `Open Diff`, `Open Artifact`;
- `Reveal File in Explorer`, `Reveal Repository in SCM`;
- `Open Agent Worktree` и `Open Worktree in New Window`;
- `Send Selection to Agent`;
- `Send Current File to Agent` с явным выбором whole file либо path/reference;
- `Send Diff/Diagnostics/Test Failure to Agent` с bounded structured context;
- `Delegate Selection/Current File/Task` с выбором provider/model, permissions,
  cwd и recipient;
- `Send Follow-up`, `Cancel Run`, `Retry Task`, `Reassign Task`;
- `Review Attention` и `Resolve Approval`.

Отправка selection включает текст, `file URI`, range, language ID и digest
версии документа. Unsaved buffer передаётся только после явного выбора: как
bounded content snapshot с меткой `unsaved`, либо после сохранения. Команда
`Send Current File` по умолчанию передаёт ссылку и релевантный диапазон, а не
неограниченное содержимое файла. Перед отправкой показываются provider,
получатель, объём контекста и файлы; секретные/исключённые пути блокируются
workspace policy.

Diff и text Artifact открываются через native read-only editor/diff tabs;
сравнение вызывает `vscode.diff(base_uri, subject_uri, title)`, а не собственный
Git/diff renderer extension. Repository state приходит от daemon для ownership
и causality, но native SCM остаётся источником IDE-представления. Extension не
подменяет Git-команды, merge UI или language server.
Diagnostic/file references из agent log становятся кликабельными и переходят к
точному URI/range. Для proposed file change extension показывает base/proposed
content, источник Agent/Task и approval actions. Approval решения отправляются
daemon и считаются принятыми только после его acknowledgement.

Daemon публикует opaque deep links на workspace/task/agent/run/artifact и
file/range, например логические `agent://...` URI. Схема не содержит credential,
raw filesystem path или исполняемую команду. Extension разрешает deep link
через daemon, проверяет workspace identity и permissions и только затем
открывает ресурс. Ссылки имеют version, bounded length и при необходимости
одноразовый short-lived token.

IPC работает через Unix domain socket на Unix и named pipe на Windows; TCP не
включается по умолчанию. Endpoint создаётся с правами текущего OS user. При
первом подключении extension выполняет challenge-response с одноразовым pairing
code из TUI/daemon; далее использует отзываемый credential из OS secret storage.
Каждый запрос содержит client ID, protocol version и workspace ID. Daemon
проверяет авторизацию на уровне действия и не доверяет extension только потому,
что оно запущено локально. Approval, write и process actions защищены от replay
request ID/idempotency key и аудитом.

Versioned daemon/extension protocol включает ProjectForest snapshot,
repository/worktree status, owner overlays, Artifact base/subject refs,
file/URI/range, commands и capability negotiation. API должен поддерживать
snapshot + ordered event subscriptions с per-stream resume cursor, чтобы
переподключение extension не создавало дублей и не теряло approvals. Версии
согласуются handshake; несовместимый клиент получает upgrade/fallback message,
а не частично работающий интерфейс. Payload bounded, большие Artifact передаются
отдельным потоковым endpoint с digest.

Для Remote SSH, Dev Containers и WSL extension определяет, где работает daemon:

- daemon в remote extension host — предпочтительный режим для remote workspace;
- local daemon + объявленное URI mapping — только после проверки workspace;
- явный disconnected/read-only snapshot mode, если маршрут недоступен.

Extension не угадывает преобразование local/remote path. URI преобразует
workspace adapter, сохраняя scheme, authority и remote identity. Pairing не
переносится между local и remote host автоматически. Порт-форвардинг и доступ
не от loopback требуют отдельной настройки и аутентификации.

Extension остаётся необязательным. Если оно не установлено, отключено,
несовместимо или daemon недоступен, TUI viewer и безопасный `code/cursor --goto`
продолжают работать. Ни одна core-функция broker, approvals или recovery не
зависит от editor API. Ошибка extension отображается как диагностическое
состояние и не завершает Run.

Daemon передаёт только typed declarative actions (`reveal`, `open`, `diff`,
`focus`, `send_context`) и URI после проверки workspace/repository/worktree
identity. Он не может прислать executable, shell command, VS Code command ID с
произвольными arguments или terminal text для автоматического выполнения.
Extension использует allowlist собственных команд и нативных API. Multi-root
разрешение проверяет authority, canonical root, symlink, nested repo, submodule
и worktree boundary перед reveal/open/send; неизвестная capability даёт явный
fallback на TUI или безопасный CLI `--goto`.

### 8.6. Compare / fan-out

Compare запускает одну неизменяемую `TaskSnapshot` одновременно на N явно
выбранных provider/model. Snapshot содержит инструкции, bounded context,
artifact refs с digest, cwd, effective permission ceiling, tool policy,
deadline и budget. После старта snapshot не изменяется: follow-up создаёт новый
compare либо отдельную Task. Это гарантирует, что участники сравниваются на
одинаковом входе, а не на последовательно накопленном контексте.

Один compare создаёт `CompareGroup` и отдельный Run для каждого участника.
Исходный Task остаётся логическим владельцем группы; правило «один активный Run»
для него ослабляется только для перечисленных Runs этой группы. Для каждого Run
сохраняются:

- provider, model и фактически использованная версия;
- queue/start/first-token/completed timestamps;
- latency до первого токена и полная длительность;
- input/output/cached/reasoning tokens, если backend их сообщает;
- фактическая и оценочная стоимость с валютой и источником тарифа;
- success, structured error, cancellation и retry history;
- Messages, Artifact и diff относительно общего base snapshot.

Неизвестные usage/cost не заменяются нулём. Метрики разных backend помечаются
как несопоставимые, если их семантика или источник различаются.

Режим завершения задаётся явно:

- `barrier=all` — ждать все terminal Runs;
- `barrier=quorum(N)` — завершить после N пригодных результатов;
- `barrier=deadline` — собрать всё, что готово к сроку;
- `partial=true` — показывать и экспортировать результаты по мере готовности.

Timeout или ошибка одного участника не скрывает успешные результаты остальных.
После достижения barrier пользователь или policy может отменить stragglers;
автоматическая отмена включается только явной настройкой и фиксируется в audit
log. Retry одного участника создаёт новый Run в той же позиции сравнения и не
подменяет исходную попытку в истории.

Fairness требует одинаковых TaskSnapshot, cwd, artifact versions, permission
ceiling, network policy и логической tool policy. Адаптер документирует
несовпадения capability: например, недоступный tool, иной sandbox или отсутствие
resume. UI показывает такие Runs как `non-equivalent`; они не участвуют в
автоматическом выборе победителя без явного разрешения. Vendor credentials,
provider system prompts и внутренние инструменты не копируются между backend.

Представление Compare содержит таблицу Runs и синхронизированный split-view:
status, first-token/total latency, tokens, cost, errors и artifacts. Text
результаты можно просматривать рядом; file changes сравниваются как diff каждого
Run относительно одного base digest, а не друг относительно друга. Итоговый
экспорт содержит snapshot digest, все результаты, ошибки и отсутствующие поля.
Автоматический «победитель» вне scope первой версии; пользователь выбирает
результат либо отправляет bounded набор результатов отдельному judge-agent.

CLI/TUI UX:

```text
/compare codex:gpt-5.5 claude:sonnet gemini:pro -- task text
/compare --barrier quorum:2 --deadline 5m --cancel-stragglers ...
```

После команды открывается preview участников, общего context/cwd, permissions,
tool policy и budget. Запуск требует подтверждения суммарного максимального
budget. В ходе выполнения `Enter` открывает выбранный Run, `Tab` переключает
table/split/diff, `c` предлагает отменить Run или оставшихся stragglers, а
`Esc` возвращает к workspace без отмены группы.

### 8.7. Broker-aware skills

Skill может объявить использование broker API и работать одинаково из model
agents разных provider без прямого доступа к vendor credentials. Skill не
запускает чужой CLI и не получает API keys: аутентифицированный вызов поступает
broker от имени текущих Agent, Task и Run, а provider adapter выбирается внутри
control plane.

Manifest broker-aware skill содержит:

- имя, версию и digest пакета;
- требуемую версию broker protocol;
- capabilities: `task.create`, `message.send`, `task.query`, `task.wait`,
  `task.cancel`, `result.collect` и при необходимости `compare.create`;
- допустимые provider/model либо capability selectors;
- максимальные depth, fan-out, concurrency, tokens, cost, duration и context;
- требуемые artifact/tool/network permissions;
- deterministic input/result schema и declared error variants.

Broker выдаёт краткоживущий scoped capability token, связанный с identity
`workspace_id + agent_id + task_id + run_id + skill_digest`. Token не может
превысить effective permission ceiling вызывающего Run, не передаётся дочернему
процессу как vendor credential и отзывается при cancel/terminal state. Каждая
операция проходит policy check, idempotency и audit.

Минимальный bus API:

- `create_task(snapshot, route, limits, idempotency_key)`;
- `send_message(recipient, content, artifact_refs, idempotency_key)`;
- `query_task(task_id)` и bounded `list_children(cursor, limit)`;
- `wait(task_ids, barrier, deadline, cursor)`;
- `cancel(task_or_run_id, reason, idempotency_key)`;
- `collect(task_ids, result_schema, cursor)`.

Ответы имеют versioned deterministic envelope: status, typed result/error,
ordered message refs, artifact refs/digests, usage/cost with unknown markers и
resume cursor. Свободный provider event не считается результатом skill, пока
адаптер/broker не валидирует его по заявленной схеме. Большие данные передаются
только Artifact refs; размер каждого вызова и суммарного собранного контекста
жёстко ограничен.

Созданные skill задачи наследуют causal chain, depth и budget родителя. Рекурсия
того же skill учитывается по `skill_digest`; manifest задаёт более строгий
предел, а workspace policy — абсолютный. Циклы зависимостей, превышение depth,
fan-out, context или budget завершаются typed error без частичного повышения
полномочий. `wait` не удерживает неограниченный ресурс: поддерживает deadline,
cancel и reconnect cursor. Завершение родителя по policy отменяет либо
осиротевшие дочерние задачи, либо передаёт их root/user inbox.

Установка skill сама по себе не разрешает broker capabilities. При первом
использовании UI показывает manifest, publisher/digest, предполагаемый fan-out,
permission ceiling и максимальный budget. Изменение manifest/digest аннулирует
ранее выданное постоянное разрешение. Недоверенные инструкции от другого агента
не могут изменить identity, route policy или ceilings skill-вызова.

### 8.8. Shared code intelligence и retrieval

Для всех provider используется один центральный vendor-neutral indexing and
retrieval service. Он владеет индексом, ACL, freshness, cache и audit, но не
model sessions. Подключение выполняют тонкие adapters:

- MCP server adapter там, где MCP transport и scoped configuration стабильны;
- native tool/plugin adapter либо CLI bridge для backend без подходящего MCP;
- broker API для TUI, daemon, skills и будущих клиентов.

Глобальный MCP или native index отдельно для каждого vendor проще локально, но
дублирует данные, расходится по freshness/ACL и делает citations несопоставимыми.
Выбран hybrid: центральный service является source of truth, а MCP/native/CLI
adapters только переводят authentication, capabilities и result schema.
Orchestrator выдаёт каждому Run short-lived scoped connection descriptor в
понятном provider формате; raw общие credentials и embedding keys агентам не
передаются.

#### Identity, freshness и incremental index

Index scope идентифицируется как `repository identity + worktree identity +
commit SHA + dirty overlay digest`. Branch name недостаточен. Индексы разных
worktree/branch не смешиваются; content-addressed storage может дедуплицировать
неизменённые blobs только с сохранением ACL.

Индекс обновляется инкрементально по content digest. Watchers, git index/ref/
worktree events и явные file-change notifications помечают shards stale и
запускают re-index. Watcher не доказывает freshness: перед citation проверяются
file identity и digest. Checkout/rebase/reset, submodule, generated output и
изменение parser/config invalidated соответствующие projections. Result
содержит indexed commit/overlay, generation, freshness и время проверки.

#### Индекс и retrieval pipeline

Hybrid retrieval объединяет lexical search, symbols/AST, definitions,
references, imports, call/dependency graph, связи code/tests, docs, manifests,
package/build scripts, CI/config и semantic/vector search. Поддержка языка
публикуется как capability matrix с parser/version и точностью symbol/reference/
call graph. Для неизвестного языка или parser failure применяется bounded
`rg`-подобный lexical fallback с явным статусом `degraded`.

ACL/filtering применяется до retrieval и повторно после graph traversal и
reranking. Pipeline объединяет lexical/symbol/semantic scores, дедуплицирует
chunks и выполняет bounded reranking. Cache key включает normalized query,
repo/worktree/snapshot, principal/policy digest, index generation и parser/
embedding/reranker versions. Ответ ограничен result/byte/token budgets.

Каждый result содержит exact `file:line[:column]`, symbol/kind, content digest,
repo/worktree/commit/overlay, index generation, freshness, retrieval evidence/
scores и bounded excerpt. Context pack состоит только из таких citations и
имеет собственный digest; непроверенная строка помечается stale/unknown.

#### API и поведение агентов

Минимальный versioned API:

- `search(scope, query, filters, budget, cursor)`;
- `symbol(scope, qualified_or_fuzzy_name, kind, budget)`;
- `references(scope, symbol_or_citation, direction, budget, cursor)`;
- `context_pack(scope, intent, seed_citations, budget, policy)`;
- `index_status(scope)` и scoped `request_index(scope)`.

Агент явно вызывает retrieval tool. Автоматическое неограниченное добавление
индекса в prompt запрещено; каждый context pack целевой, bounded и учитывается
в token budget. Cursors привязаны к index generation; stale cursor возвращает
typed error.

Broker публикует `index_building`, `index_ready`, `index_stale`,
`index_degraded` и `index_failed` с scope/generation. При недоступном service
adapter возвращает typed unavailable, не зависает и не подменяет retrieval
скрытым vendor search. Run может явно ждать, использовать lexical fallback или
продолжить без индекса; выбор фиксируется в evidence и compare fairness.

#### Security, privacy и эксплуатация

Indexing пересекает workspace ACL, Run ceiling и file policy. Secret/excluded
paths, credentials, binary, generated и vendor directories имеют отдельные
правила; `gitignore` не является secret policy. Запрещённый content не попадает
в chunks, embeddings, cache, logs или excerpts. ACL проверяется для каждой
citation и graph traversal: знание symbol/path не даёт доступа.

Indexed content является untrusted data. Comments, docs, tests и tool-like text
оборачиваются как quoted evidence с provenance и не становятся system/tool
instructions. Prompt injection, Unicode/encoding obfuscation и инструкции в
generated/vendor content не могут обойти schema, ACL и tool policy.

Embeddings backend выбирается отдельно: local model либо внешний provider.
Внешняя отправка требует policy по classification, tenant/region, retention и
opt-in; secrets и запрещённые paths не отправляются. Model/version/dimensions и
normalization digest входят в projection identity. Lexical/symbol retrieval
остаётся доступным при отключённых embeddings.

Service экспортирует bounded audit/metrics: principal/Run, scope/query digest,
methods, citation count, bytes/tokens, cache hit, freshness, stage latency,
parser/embedding/reranker versions и typed error. Raw query/excerpts сохраняются
только по отдельной redaction/retention policy.

## 9. Интеграция с agent-orchestrator

`agent-orchestrator` рассматривается как внешний authoritative control plane и
единственный writer orchestration state. Он владеет Task/Run lifecycle,
dependency/fan-out/compare policy, permission ceilings, budgets, retry,
approval audit, append-only journal, outbox, idempotency и recovery.
`codex-claude-mode` не дублирует эту БД: он остаётся TUI/operator client,
хранит только ephemeral UI state и на переходном этапе может быть provider
gateway для уже реализованного Codex app-server transport.

Provider adapter владеет внешними session/run ID, capability discovery,
launch/resume/observe/cancel/recover, применением effective permissions и
нормализацией provider events. Orchestrator хранит только metadata/digest/ref
Artifact; байты больших logs/diff остаются в ограниченном artifact storage.

Локальная граница — versioned JSONL protocol по stdio либо Unix socket. Envelope
содержит schema/event/request ID, sequence, causation/correlation, workspace,
task, run и agent ID; mutating command содержит idempotency key и expected
revision. Клиент подписывается с replay cursor. Неизвестные версии, события,
разрывы sequence и конфликтующие повторы fail closed. Сетевой transport не
является следствием выбора этого локального протокола.

Минимальные команды: `workspace.snapshot`, `task.create/cancel/retry/reassign`,
`compare.create`, `message.send`, `run.cancel`, `approval.resolve`,
`agent.permissions.set`, `subscribe(from_sequence)`. События включают Agent,
Run и Task lifecycle, provider observation, Approval, Message, Artifact, Usage
и FanOut transitions.

Миграция выполняется без big bang:

1. общие schema fixtures и нейтральные Rust view types, direct Codex UX без
   изменений;
2. read-only stdio bridge и snapshot/replay в TUI;
3. shadow gateway: Codex events журналируются, но orchestrator не управляет
   процессом;
4. отдельно одобренные live `observe`, затем `launch/cancel/approval` через
   outbox;
5. Claude pilot и compare; далее provider gateway выносится из TUI в
   долгоживущий adapter/sidecar.

До завершения миграции сохраняется `--backend direct-codex`; существующие
Codex sessions импортируются как read-only discovered agents, а внутренние
orchestrator ID не заменяются внешними thread ID.

Важно: это ТЗ не изменяет соседний репозиторий `agent-orchestrator` и не даёт
разрешения на реализацию его public contract. Его `AOR-ACCESS-1`, provider/live
границы и любые изменения публичного протокола требуют отдельного явного
решения пользователя.

### 9.1. ADR-кандидаты удалённого доступа

Рассматривается конечный набор вариантов:

- **A — SSH stdio (рекомендуемый первый вариант):** клиент запускает либо
  подключает protocol process через SSH; отдельный listening port отсутствует,
  SSH даёт host/user authentication и шифрование.
- **B — loopback Unix/TCP + SSH tunnel:** локальный daemon слушает только
  loopback/Unix socket, SSH отвечает за reachability. Удобнее нескольких
  клиентов, но требует lifecycle и защиты локального endpoint.
- **C — HTTPS/WSS service:** mTLS для host/workload, OIDC для пользователей,
  tenant/project RBAC и отдельный audit service. Нужен для web/multi-user, но
  существенно расширяет trust boundary и runtime dependencies.
- **D — overlay network:** Tailscale/WireGuard-подобная сеть предоставляет
  только reachability; поверх неё всё равно обязательны application identity,
  RBAC, replay protection и audit. Overlay не считается авторизацией.

Для любого варианта approval связывается с authenticated operator identity,
role/policy revision, digest исходного запроса, выбранным решением, временем и
causal event; stale/повторное решение отклоняется. Read-only наблюдение и
mutating действия имеют разные права. Reconnect использует monotonic replay
cursor, bounded retention, backpressure и snapshot+delta при устаревшем cursor.

Remote files открываются встроенно как явно помеченные snapshots либо через
проверенный provider URI/local-root mapping. Внешний редактор не получает
притворный локальный рабочий файл; export требует отдельного действия. Тесты
угроз покрывают credential theft, confused deputy, tenant/workspace escape,
symlink/path traversal, event injection, replay, stale approvals, log/Artifact
leakage, cursor truncation, denial of service и небезопасное обновление.

### 9.2. First-class Linux и macOS platform contract

Linux и macOS являются обязательными first-class supported platforms для всех
функций local MVP и каждого PoC vertical slice. Функция не считается готовой,
если она реализована или протестирована только на Linux. Поддерживаются
`x86_64` и `aarch64` там, где provider CLI и зависимости выпускают совместимые
artifacts; отсутствие конкретной комбинации отражается capability probe и
release matrix, а не обнаруживается после запуска.

| Область | Linux | macOS | Общее требование |
|---|---|---|---|
| Process lifecycle | POSIX spawn, process groups, signals | POSIX spawn/process groups с отличиями signal/process behavior | Adapter supervisor завершает полное известное дерево, отличает graceful cancel от force kill и создаёт один terminal outcome. |
| Terminal | termios/raw/alternate screen | termios/raw/alternate screen | Panic, signal, child exit и normal quit восстанавливают cursor, raw mode и alternate screen. |
| IPC/transport | stdio, Unix sockets | stdio, Unix sockets | Одинаковые versioned envelopes/replay; socket path length, permissions и stale endpoint проверяются platform adapter. |
| Paths/files | case-sensitive обычно, arbitrary Unix bytes | часто case-insensitive preserving, Unicode normalization отличается | Внутренние OS paths не обязаны быть UTF-8; wire/display encoding не меняет identity. Canonical/resolved identity учитывает symlink, mount и filesystem case rules. |
| File watching | inotify | FSEvents | Общая watcher abstraction с coalescing/overflow handling и bounded polling fallback; event не считается доказательством freshness. |
| Editors | `code`, `cursor`, `vim`/terminal argv | `code`, `cursor`, `vim`/terminal argv | Direct argv spawn без shell. macOS `open -a <explicit app> --args ...` допускается только как выбранный безопасный fallback, не через произвольную shell-строку. |
| Isolation | bwrap/namespaces/seccomp/cgroups v2, rootless containers | provider-native, Seatbelt/`sandbox-exec` только при доказанной availability, container VM или VM/remote executor | Одинаковая requested policy semantics, но честные effective capabilities/evidence; unsupported/degraded никогда не ослабляются молча. |
| Containers | Docker Engine/rootless Docker/Podman с host Linux kernel | Docker Desktop/Podman machine через Linux VM | Различия VM, networking, mounts, UID mapping и performance входят в capability/evidence и compare equivalence. |
| Optional desktop | clipboard/desktop notifications capability-gated | clipboard/Notification Center capability-gated | Ошибка или отсутствие интеграции не влияет на approvals, logs или lifecycle. |

Process invocation в product/runtime/tests формируется как executable + argv,
cwd и explicit environment без Linux-only shell syntax, `/proc` assumptions,
GNU-only flags или hard-coded `/bin/bash`. Shell-specific функция сначала
определяет фактический shell/capability. Нельзя полагаться на одинаковые `sed`,
`stat`, `readlink`, `mktemp`, signal numbers или command output BSD/GNU variants.

Path и permissions layer использует native path representation до границы
protocol. Non-UTF8 path на Linux получает lossless encoded wire form или typed
unsupported, но не lossy alias; macOS Unicode/case behavior проверяется на
фактическом volume. До destructive/write action повторно проверяются resolved
root, symlink и file identity для защиты от TOCTOU. Unix socket создаётся в
private directory текущего user, с проверкой owner/mode и безопасным удалением
только доказанно принадлежащего stale endpoint.

Linux isolation может обеспечить namespaces/bwrap, seccomp, cgroups и rootless
container. macOS implementation обязана probe availability и пригодность
Seatbelt/`sandbox-exec`; platform API может быть deprecated/restricted и не
считается гарантированным boundary без evidence. External Full Disk Access,
Automation/TCC permissions и container/VM mounts показываются отдельно:
orchestrator не может обещать их запрет, если OS уже выдала доступ host process.
При отсутствии требуемой границы Run блокируется либо после явного решения
работает с красным `UNSANDBOXED/DEGRADED` статусом.

SSH stdio является первым remote transport и обязан работать с Linux и macOS в
обеих ролях client/host, не предполагая одинаковую remote shell environment.
Handshake передаёт OS/arch/path/isolation/provider capabilities; protocol
остаётся platform-neutral.

SQLite migrations, event journal и snapshots переносимы между поддерживаемыми
платформами и не сериализуют platform-native handles. Storage layer документирует
locking, concurrent readers/writer, `fsync` файла и parent directory, atomic
rename assumptions и recovery после crash; durability probes выявляют volume,
где требуемые semantics не гарантированы. Temp/state/cache paths выбираются
через platform directories и сохраняют permissions.

CI обязательно включает Linux и macOS: unit, integration, golden protocol,
adapter capability probes и TUI snapshot tests. Golden fixtures одинаковы, а
platform evidence snapshots разделены. Tests покрывают signals/process groups,
terminal restoration, Unix sockets, file watching overflow/poll fallback,
case/Unicode/non-UTF8 paths, symlinks, atomic recovery и safe editor argv.
Packaging выпускает platform/architecture artifacts с checksum и provenance;
code signing/notarization и marketplace packaging являются последующим release
этапом и не должны блокировать исходный PoC.

Windows не входит в MVP. Wire protocol, IDs, events, path/URI abstraction и
storage schema остаются переносимыми, чтобы позднее добавить named pipes, Job
Objects/AppContainer/WSL без breaking protocol change.

## 10. Хранение и восстановление

SQLite с append-only event journal и материализованными состояниями. Таблицы:
workspaces, providers, agents, provider_sessions, tasks, runs, messages, events,
approvals, artifacts, usage_samples и process_runs.

После crash выполняется reconciliation БД, процессов и provider sessions.
Orphan process не присоединяется автоматически без проверки provider ID, cwd и
сохранённого run token. Terminal Run не возвращается в активное состояние от
позднего события.

### 10.1. Observability, metrics и traces

Локальный dashboard является первым consumer observability; optional
OpenTelemetry-compatible export добавляется без обязательного collector.
Prometheus endpoint/gateway рассматривается позже и по умолчанию не слушает
network interface. Metrics должны сохранять признак unavailable/unknown:
неизвестные tokens, cost, context или latency не экспортируются как ноль.

Логические dimensions: workspace, provider, model, agent, task, run, attempt и
role. Raw IDs, пути, имена пользователей и prompts не становятся metric labels.
Высококардинальные identity связываются через traces/events или bounded opaque
buckets; exporter применяет allowlist labels, cardinality budgets и overflow
series. Набор метрик включает:

- lifecycle counters/gauges и число active/idle/waiting/closed/archived/unread
  sessions, их age и health;
- queue, dependency wait, approval wait, run и total duration, TTFT;
- input/output/cache/reasoning tokens, context occupancy, cost/currency и
  availability/source каждого значения;
- tool call latency, outcomes/errors, retries, cancellation, stalled,
  heartbeat/lease expiry;
- approval latency, type и outcome без sensitive details;
- bus delivery/ack/error/duplicate и consumer lag;
- sandbox/backend process CPU, RAM, IO, network и доступные cgroup/container
  ограничения с явным признаком отсутствия telemetry;
- RAG index age/freshness/build latency, query latency/errors, result counts и
  bounded recall proxies без сохранения query/code;
- remote clients, reconnects, connection age/lag и revision conflicts.

Distributed trace переносит `trace_id`, `span_id`, correlation и causation через
Task delegation, Message bus, broker, adapter и provider callbacks. Отдельные
spans покрывают queue, policy/approval, adapter call, tool и artifact transfer;
provider trace ID хранится как namespaced link. Structured logs содержат
timestamp, severity, event ID, trace/span, stable internal refs, event type и
typed outcome. Logs не являются источником истины вместо event journal.

Prompts, responses, source code, diffs, secret values, raw tool arguments и
filesystem paths не попадают в metrics/traces/logs по умолчанию. Diagnostic
payload разрешается только opt-in policy, проходит field redaction, size limit
и sampling. Retention bounded отдельно по signals; доступ и export
observability защищены RBAC и аудитируются. Sampling сохраняет все security/
approval/terminal errors, но не отменяет redaction.

SLO задаются по capability и deployment: broker availability, dispatch delay,
approval delivery, event/bus lag, reconnect recovery, terminal-event integrity
и bounded resource usage. Alerts включают lease loss, stalled Runs, approval
backlog, retry/error spike, queue saturation, budget/cost anomaly, RAG stale,
client reconnect storm и exporter failure. Alert показывает scope и missing
telemetry, не объявляет систему healthy только из-за отсутствия samples.

### 10.2. Token analytics и model/provider recommender

Usage нормализуется в input/output/cache-read/cache-write/reasoning tokens,
context limit/occupancy и cost только когда provider это сообщает. Каждое поле
имеет `reported | estimated | unavailable`, source и resolved provider/model/
version. Price tables versioned по effective date/currency; историческая цена
не пересчитывается новой таблицей без отдельного сценария. Аналитика доступна
по workspace, Task/type, provider/model/version, LogicalAgent, Run/attempt и
accepted result; стоимость результата включает failed attempts, compare Runs,
reviews и rework, а не только финальный успешный ответ.

Budgets поддерживают hard/soft limits, forecast до завершения и anomaly events
по cost/token/context. Unknown usage не считается нулём и снижает полноту
forecast. Task taxonomy включает явные tags `coding`, `review`, `debug`,
`research`, `planning`, `test`, `docs`, `security`, `large-context` и расширения.
Classifier может предложить tags с confidence/evidence, но остаётся advisory;
пользовательский tag и audit history не переписываются молча.

Recommender обучается только на собственных разрешённых evidence: success и
user acceptance, TTFT/total latency, total accepted-result cost, retries,
human correction/rework, context fit и наличие required capability/sandbox.
Он предлагает Pareto-варианты `fastest`, `cheapest`, `best-confidence`, показывая
выборку, период, resolved versions, confidence, missing/censored observations и
причины. Cold start использует capability/price/declared-context rules, а не
притворяется learned recommendation. Рекомендация никогда не запускает route
молча: user override сохраняется; обучение/A-B `/compare` только opt-in.

Оценка контролирует vendor imbalance, смену model versions, малые выборки,
survivorship/censored failures и разные permission/tool условия. Несопоставимые
Runs не объединяются без stratification. Prompts/code/secrets не нужны для
агрегации; granular history имеет bounded retention/RBAC, а долгосрочные данные
анонимизируются/агрегируются с audit удаления.

## 10.3. Landscape: build, buy, integrate

| Компонент | Что уже даёт | Решение |
|---|---|---|
| [OpenAI Agents SDK](https://openai.github.io/openai-agents-python/multi_agent/) | Handoffs, agents-as-tools, parallel runs, sessions и tracing; сериализуемый [RunState/HITL](https://openai.github.io/openai-agents-python/human_in_the_loop/) | Интегрировать в OpenAI-native workflows при необходимости; не использовать как cross-vendor broker, потому что handoff принадлежит одному SDK run. |
| [Claude Managed Agents](https://platform.claude.com/docs/en/managed-agents/sessions) и Agent SDK | Agent/Environment/Session, lifecycle, изменение tools/MCP/permissions в session и [webhooks](https://platform.claude.com/docs/en/managed-agents/webhooks) | Отдельные local Claude Code и remote Managed Agents adapters; vendor session остаётся binding, не canonical Task. |
| [Gemini CLI](https://github.com/google-gemini/gemini-cli/blob/main/docs/core/subagents.md) / [ADK](https://google.github.io/agents-cli/guide/templates/) | Subagents, policies, sandbox, hooks; ADK публикует A2A и поддерживает sessions/artifacts | Интегрировать adapter и A2A edge; не переносить CLI config/state в доменную модель. |
| [Grok](https://docs.x.ai/build/features/skills-plugins-marketplaces) | CLI subagents, skills/plugins/hooks, MCP; API поддерживает [remote MCP](https://docs.x.ai/developers/tools/remote-mcp) и WebSocket | Интегрировать capability-gated CLI/API adapter; approvals обеспечивает наш policy layer, если backend их не поддерживает. |
| [AutoGen](https://microsoft.github.io/autogen/stable/user-guide/agentchat-user-guide/tutorial/teams.html) | Team patterns, handoffs, termination и [save/load state](https://microsoft.github.io/autogen/stable/user-guide/agentchat-user-guide/tutorial/state.html) | Заимствовать team/termination patterns; не принимать group chat runtime за durable control plane. |
| [LangGraph](https://langchain-ai.github.io/langgraph/index.html) | Graph checkpoints, persistence, streaming и [resumable interrupts](https://langchain-ai.github.io/langgraph/how-tos/human_in_the_loop/breakpoints/) | Возможный workflow backend; заимствовать checkpoint/idempotency semantics, не заменять им существующий orchestrator. |
| [CrewAI](https://docs.crewai.com/) | Crews/flows, persistence, HITL, knowledge и observability | Поздний adapter/A2A integration; не core runtime. |
| [Temporal](https://temporal.io/ai/agentic-ai) | Durable workflows/activities, retries, timers, long waits и parallelism | Опциональный enterprise durability backend; слишком тяжёл для local MVP. |
| [NATS JetStream](https://docs.nats.io/nats-concepts/jetstream) / [Redis Streams](https://redis.io/docs/latest/develop/data-types/streams/) | Готовые persisted network streams, replay, acknowledgements и consumer groups | Не нужны для local PoC. Рассмотреть после измерений remote scale; at-least-once transport не становится source of truth. |

Ни один из рассмотренных продуктов не объединяет управление живыми локальными
coding CLI разных вендоров, durable dependencies с немедленным wake-up,
унифицированные approvals/effective sandbox и единый TUI/IDE/remote UX. Это
остаётся уникальной областью продукта.

Сознательно не разрабатываются: собственный graph DSL, vector database, trace
backend, IDE file tree/editor/SCM/diff, новый discovery protocol и второй Rust
broker/database. Используются provider APIs, готовое vector/search storage,
OTLP backend, VS Code native UI, A2A Agent Card и authoritative Python
`agent-orchestrator` соответственно.

## 11. Этапы реализации

Порядок поставки вынесен в [ROADMAP.md](ROADMAP.md). Он определяет фазы `P0`–`P8`,
exit gates `G0`–`G8`, critical path, parallel lanes и release slices. Статус и
зависимости атомарных задач хранятся только в [TODO.md](TODO.md); этот раздел не
дублирует их и остаётся кратким нормативным summary.

Critical path: baseline freeze → shared golden contract → read-only stdio
bridge → Codex shadow parity → поштучное включение live Codex capabilities →
Claude как второй provider → bounded compare и durable bus. На этом заканчивается
core multi-provider MVP. IDE, observability, retrieval, chat/SaaS adapters и
remote access поставляются отдельными opt-in slices после соответствующих
security gates.

Linux/macOS parity проверяется в каждом vertical slice. Изменение public
contract `agent-orchestrator`, включение live capability и первый network
endpoint имеют отдельные stop gates и не следуют автоматически из этого ТЗ.

## 12. Критерии приёмки целевой системы

Ниже сохранён полный каталог acceptance criteria, включая post-MVP integrations.
Граница core MVP и критерии конкретного release slice определяются gates в
[ROADMAP.md](ROADMAP.md) и атомарными задачами в [TODO.md](TODO.md); наличие
критерия в этом каталоге не делает optional функцию зависимостью core MVP.

- Codex и Claude одновременно работают в одном workspace.
- Linux и macOS обеспечивают одинаковое direct Codex read bridge, session
  create/resume, agent tree/log/composer, editor navigation и approval behavior;
  platform-specific различия видимы только как capability/evidence.
- На Linux и macOS cancel завершает правильную process group, а normal exit,
  panic и handled signals восстанавливают terminal/raw/alternate-screen state.
- Unix socket/stdio reconnect, portable state migrations, locking/fsync/atomic
  recovery и watcher polling fallback проходят integration tests на обеих OS.
- `code`, `cursor` и настроенный `vim` запускаются безопасным argv; macOS
  `open -a` используется только как явный fallback без shell interpolation.
- Telegram/Slack без настроенной capability работают notification-only или
  явно unsupported; chat никогда не получает неразрешённый diff/file/secret.
- Approval из Telegram/Slack принимается только для paired principal, точного
  digest/revision и неистёкшего запроса; stale/replay и проигравший в гонке с
  TUI клиент получают conflict без второго решения.
- Mock Telegram Bot API и Slack Events/Socket/interactivity tests доказывают
  signature/secret verification, RBAC/channel allowlist, escaping, rate-limit
  retry, batching/dedupe, durable delivery/ack и отсутствие tokens в logs.
- Jira/Confluence mock MCP refs сохраняют canonical external ID/URL/version/
  digest/freshness как Task context/Artifact, но не заменяют canonical Task.
- Read-only MCP profile не вызывает mutation; create/update/comment/transition
  требуют exact digest/version approval, а stale/replayed request отклоняется.
- Все provider adapters получают эквивалентный allowlisted MCP profile либо
  честный degraded/unsupported status; OAuth secrets отсутствуют в prompts,
  configs, journal, logs и fixtures.
- Bounded fetch, redaction и tenant ACL предотвращают whole-space ingestion и
  data leak; malicious Jira/Confluence text остаётся untrusted evidence и не
  превращается в instruction/tool call.
- Требуемая isolation policy не ослабляется на macOS из-за отсутствия Linux
  namespaces/cgroups: capability становится unsupported/degraded и Run требует
  отказа либо явного красного unsandboxed approval.
- Задача передаётся между provider и возвращается правильному отправителю.
- UI показывает parent, provider, task, status, effective permissions и alerts.
- Approval фонового агента виден сразу и не уничтожает UI state.
- После рестарта задачи/дерево восстанавливаются без дублей.
- Упавший Run можно повторить другим provider без потери history/artifacts.
- Циклы и превышение depth/fan-out отклоняются.
- Cancel завершает backend process и создаёт ровно один terminal event.
- Retry создаёт новый Run; старые попытки остаются в истории.
- Дочерний агент не повышает permissions и не получает чужие credentials.
- Unknown provider events и unsupported capabilities fail closed.
- Usage/cost totals не трактуют неизвестные значения как ноль.
- File references из log, diff и Artifact открываются на правильной строке
  относительно cwd исходного Run и возвращают пользователя на прежний scroll.
- Viewer показывает текст с номерами строк и syntax highlighting, а large,
  binary, invalid UTF-8, missing и symlink files обрабатывает ограниченно и
  явно, без зависания или неявного выхода за разрешённый root.
- VS Code/Cursor получают точный `file:line:column`; путь с пробелами и shell
  metacharacters не приводит к выполнению дополнительных команд.
- Для remote workspace `/edit` использует объявленный URI/mapping либо явно
  недоступен; remote snapshot не выдаётся за редактируемую рабочую копию.
- `read_only` разрешает просмотр, не повышает permissions агента и явно
  маркируется при передаче файла внешнему редактору.
- Extension после pairing показывает те же Agents/Tasks/status/approvals, что и
  TUI; reconnect по event cursor не теряет и не дублирует события.
- `Send Selection` передаёт точный URI/range/digest и требует явного решения для
  unsaved content; excluded/secret paths отклоняются policy.
- Native diff/Artifact tabs и deep links открывают правильный workspace и
  позицию, не раскрывая credential или произвольную команду в URI.
- IPC недоступен другому OS user, несовместимая protocol version fail closed, а
  отключение extension не нарушает TUI viewer и `--goto` fallback.
- Remote extension host не смешивается с local daemon без явного проверенного
  URI mapping и отдельного pairing.
- TUI не реализует полноценные Explorer/SCM/Search/editor/terminal; без
  extension доступны минимальный viewer и CLI `--goto`, а с extension файлы,
  Search, Git, diff, diagnostics и tests открываются нативными VS Code/Cursor API.
- Multi-root ProjectForest корректно различает repositories, nested repos,
  submodules и per-Agent/Run worktrees; UI показывает branch/HEAD/dirty/
  ahead-behind/conflicts и ownership без пересечения symlink/root boundaries.
- Compare Artifact привязан к repository/worktree и base/subject SHA либо dirty
  overlay digest; `vscode.diff` получает правильные стороны, несовместимые базы
  явно маркируются.
- Extension reveal/open/new-window, CodeLens/context menus и decorations работают
  через capability-gated typed protocol; daemon не может выполнить произвольную
  IDE-команду, shell или terminal input.
- Cursor без нужного API получает заявленный degraded mode и безопасный
  `--goto`; reconnect subscriptions/cursors не дублируют Git/owner events.
- `/compare` передаёт каждому Run один и тот же immutable TaskSnapshot digest,
  cwd, permission ceiling и tool policy; известные различия capabilities явно
  помечаются до запуска и в результате.
- Compare отображает partial results, корректно соблюдает all/quorum/deadline
  barrier и позволяет отменить stragglers, не теряя уже полученные результаты.
- Для каждого compare Run независимо сохраняются first-token/total latency,
  tokens, cost, success/error и artifacts; неизвестные значения не становятся
  нулевыми, а diff строится относительно общего base digest.
- Broker-aware skill без vendor credentials создаёт mixed-provider задачи,
  отправляет сообщения, ожидает/собирает результаты и отменяет работу только в
  пределах scoped identity, permission ceiling, depth и budget.
- Повтор bus-запроса с тем же idempotency key не создаёт дубли, result envelope
  проходит заявленную schema, а context/artifacts остаются bounded.
- Рекурсивный skill, цикл и превышение depth/fan-out/context/cost завершаются
  typed policy error; изменение skill digest требует нового разрешения.
- `@` autocomplete различает agent/provider:model/role/channel, поддерживает
  fuzzy keyboard selection и создаёт typed stable reference, не зависящий от
  display name; rename, narrow rendering и copy/paste не меняют адресата.
- Execution target и result recipients сохраняются раздельно; естественная
  фраза остаётся в prompt, но доставка определяется structured metadata без LLM
  parsing, а неоднозначная/устаревшая ссылка блокирует отправку.
- Несколько recipients получают независимые delivery/ack; collector/channel
  routing дедуплицируется, unread отображается в Inbox, а offline/closed
  recipient обрабатывается заданным durable fallback без утечки permissions.
- Ключевые mixed-provider, approval, narrow layout, stalled/failed и recovery
  состояния, а также file viewer и editor launch preview покрыты integration и
  snapshot tests; extension contract покрыт IPC/auth/reconnect tests.
- Codex и Claude получают одинаковые digest-bound citations из одного index
  generation через разные adapters; raw service credentials им не выдаются.
- Branch/worktree/dirty overlay не смешиваются; commit/checkout/rebase и
  изменение parser/embedding config корректно invalidated индекс и cursors.
- `search`, `symbol`, `references`, `context_pack` и `index_status` соблюдают
  ACL, result/token limits, freshness и exact `file:line` + content digest.
- Недоступный parser даёт явно degraded lexical fallback, недоступный service —
  typed unavailable; ни один режим не вызывает automatic context injection и
  не выдаёт stale result за текущий код.
- Secret/excluded content не попадает в chunks, embeddings, cache, logs или
  cross-provider results; prompt injection остаётся quoted untrusted evidence.
- Session lifecycle сохраняет stable internal identity, Task ownership,
  per-client draft/scroll/unread; unsupported resume/fork/import видны явно, а
  Task reassignment создаёт новый Run вместо vendor-session migration.
- Crash/race tests доказывают отсутствие duplicate process/Turn/approval при
  reconnect, stale revisions, leader lease loss и orphan adoption.
- Bulk session actions применяют revision-bound preview; delete/export
  соблюдают approval, RBAC, retention/legal hold, redaction и Artifact links.
- Metrics выдерживают cardinality budget, unknown остаётся unknown, а privacy
  tests подтверждают отсутствие prompts/code/secrets/raw IDs в labels/logs.
- Load tests покрывают event/bus lag, heartbeats, session age, tool/approval
  latency и resource metrics; traces сохраняют correlation/causation через
  mixed-provider delegation и reconnect.
- Usage ledger фиксирует exact model version, provenance и историческую цену;
  accepted-result cost включает failures/reviews/rework, forecasts обозначают
  estimated/unavailable данные.
- Recommender calibration/bias tests покрывают cold start, малые и censored
  samples, vendor imbalance и model-version drift; UI объясняет sample size,
  confidence/Pareto trade-off и никогда не выполняет silent auto-routing.

## 13. Открытые продуктовые решения

1. Только локальный orchestrator в MVP или сразу daemon/remote UI?
2. Claude/Gemini подключать только через CLI либо допустить SDK/API transport?
3. Grok: CLI при наличии или API считать основным официальным backend?
4. Нужны ли отдельные git worktree для каждого write-capable агента по умолчанию?
5. Должен ли результат автоматически попадать родителю или сначала требовать
   пользовательского просмотра?
6. Какие default limits: concurrency, depth, fan-out, cost, tokens и timeout?
7. Нужен ли auto-routing в первой публичной версии или только explicit provider?
8. Какая политика хранения logs/artifacts и срок удаления?
9. Нужен ли общий MCP/tool host или каждый CLI использует только свои tools?
10. Должен ли Main быть нейтральным broker либо всегда реальным model-agent?
11. Какие remote URI schemes официально поддерживаются для VS Code/Cursor?
12. Разрешать ли opt-in временный export remote Artifact для внешнего просмотра?
13. Поставлять extension отдельно через marketplaces или вместе с daemon?
14. Должен ли extension поддерживать web-версию VS Code без локального IPC?

## Приложение A. Обоснование архитектурных решений

Это приложение фиксирует rationale текущего направления. Детальные контракты и
ограничения остаются в разделах 3, 8–11.

1. **`agent-orchestrator` — authoritative control plane.** Task, Run,
   dependency transition, journal, inbox/outbox и policy decision требуют
   одного deterministic writer: иначе два процесса смогут одновременно
   разблокировать зависимость, повторить side effect или разрешить approval.
   Python-компонент уже владеет этими инвариантами, поэтому перенос их в TUI не
   создаёт ценности.

2. **`codex-claude-mode` — thin operator client.** TUI отвечает за projection,
   ввод, навигацию и attention, но не за долговечность выполнения. Его закрытие
   не должно останавливать работу или менять Task state; переходный Codex
   gateway допускается только до выделения долгоживущего adapter.

3. **Provider adapters — отдельные процессы/модули.** Они переводят lifecycle,
   capabilities, approvals и usage конкретного backend в общие события, не
   протаскивая vendor-типы в broker. Сбой или обновление одного CLI не должен
   требовать обновления authoritative state machine.

4. **Codex подключается напрямую через app-server.** Это официальный rich-client
   API с Thread/Turn/Item, streaming, resume/fork, usage и server-initiated
   approvals; scraping TUI потерял бы типы и lifecycle guarantees:
   <https://github.com/openai/codex/blob/main/codex-rs/app-server/README.md>.

5. **ACP ограничен границей editor ↔ coding agent.** Он полезен для запуска и
   взаимодействия IDE с совместимым агентом, но не заменяет durable Task graph,
   policy или multi-provider bus:
   <https://agentclientprotocol.com/get-started/architecture>.

6. **MCP используется для tools, RAG и bus-aware skills.** MCP стандартизирует
   tools/resources/prompts и local/remote transports, поэтому через него модели
   получают code intelligence и scoped bus calls. Источником истины он не
   становится: <https://modelcontextprotocol.io/docs/learn/architecture>.

7. **A2A используется для внешней federation.** Agent Card, asynchronous Task,
   Message, Artifact, streaming и push подходят для связи с внешними агентами,
   но не определяют наши leases, dependency transitions и approval ownership:
   <https://a2a-protocol.org/latest/specification/>.

8. **AG-UI остаётся необязательной web/frontend проекцией.** Его event model
   можно отобразить поверх control-plane API после стабилизации local protocol;
   он не нужен provider adapters и не должен влиять на journal schema:
   <https://docs.ag-ui.com/concepts/architecture>.

9. **Observability экспортируется через OpenTelemetry/OTLP.** Готовые GenAI
   semantic conventions покрывают model, operation и token usage, сохраняя
   совместимость с разными backends. Prompt/code content остаётся opt-in из-за
   чувствительности: <https://opentelemetry.io/docs/specs/semconv/registry/attributes/gen-ai/>.

10. **Code intelligence собирается из LSP, SCIP и tree-sitter.** LSP даёт live
    language semantics, SCIP — переносимый code index, tree-sitter — bounded
    parsing; вместе с lexical search они закрывают основу без собственного
    языка анализа: <https://microsoft.github.io/language-server-protocol/>,
    <https://github.com/sourcegraph/scip>, <https://tree-sitter.github.io/tree-sitter/>.

11. **Мы не создаём IDE или Git UI.** VS Code/Cursor уже предоставляют native
    Explorer, Search, SCM, diff, editor, terminal, multi-root workspace и remote
    UX. Extension добавляет только Agents/Tasks/Attention/Artifacts и вызывает
    нативные API; TUI сохраняет минимальный read-only fallback.

12. **Мы не создаём graph DSL, vector DB или trace backend.** Это зрелые
    инфраструктурные категории без уникального преимущества продукта. При
    необходимости используются LangGraph как optional workflow backend,
    существующее search/vector storage и OTLP-compatible collector; сравнение
    приведено в разделе 10.3.

13. **MVP начинается с Codex и Claude.** Два реально отличающихся coding-agent
    backend достаточны, чтобы проверить нейтральность schema, permissions,
    recovery и compare. Gemini/Grok добавляются только после прохождения тех же
    fixtures, иначе четыре незрелых adapter скроют ошибки core contract.

14. **Первый bridge строго read-only.** Snapshot/subscription позволяет сверить
    Python и Rust projections без риска двойного launch, cancel или approval.
    Live capability включается отдельно лишь после replay, deduplication и
    crash tests.

15. **Начальный transport — versioned JSONL stdio.** Он прозрачен, легко
    записывается в golden fixtures и не требует network identity. Тот же
    envelope затем переносится на Unix socket; sequence, replay cursor,
    idempotency и hard payload limits принадлежат protocol, а не transport.

16. **Direct-Codex сохраняется как временный fallback.** Он обеспечивает
    работающий продукт и эталон для shadow comparison, пока orchestrated path
    не доказал parity. Fallback удаляется только после наблюдаемой совместимости,
    а не ради архитектурной чистоты.

17. **Remote access откладывается до ADR.** Первый эксперимент — SSH stdio с
    существующим protocol. HTTPS/WSS, host enrollment, mTLS/OIDC, AG-UI и
    NATS/Redis/Temporal требуют отдельного threat model и измерений; ранний
    network daemon преждевременно закрепил бы identity и tenancy ошибки.

18. **Уникальная область продукта сохраняется узкой.** Среди рассмотренных
    решений нет единого слоя для живых локальных coding CLI разных вендоров,
    durable dependencies с push wake-up, unified approvals/effective sandbox и
    общего TUI/IDE/remote управления. Именно это строится; стандартные IDE,
    retrieval, telemetry и protocol primitives интегрируются.

19. **PoC идёт вертикальными доказуемыми шагами.** Порядок: shared golden
    fixtures → Python read-only stdio → neutral Rust projections → Codex shadow
    observation → отдельно одобренный live `Observe` → Claude adapter →
    immutable compare → scoped bus/federation → IDE/OTLP → remote ADR. Каждый
    шаг добавляет одну границу ответственности и имеет rollback до предыдущего.
