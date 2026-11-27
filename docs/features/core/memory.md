# 上下文、长期记忆与每日归纳

[返回功能索引](../README.md) · [返回文档总索引](../../README.md) · [本模块索引](README.md)

**当前状态：已实现（代码入口）**。此判断基于当前 `main` 的代码入口静态核对；真实模型、NapCat 与外部 API 需在目标环境单独验收。

> 回复快照、人物简档、话题记忆、每日摘要与记忆列表。

## 实际行为

- `Context Builder` 在数据库一致性读取中生成回复快照，固定回复锚点、引文、近期消息及相关人物资料；本轮 Decision 与普通回复共享快照。
- 人物简档与话题记忆保留来源和时间；每日归纳走后台侧路，整理对话摘要、人物事实与关系事件。
- 当前记忆检索主要是词项匹配，并非向量数据库；上下文预算主要按字符数近似，不是严格 token 预算。
- 管理页可查看记忆和摘要，但查看、归纳、验证事实是不同步骤。

定向请求的图片和引用可进入视觉观察，复用缓存、成本闸和并发限制；原消息、本人资料、互动关系共同提供语境，观察再进入回复快照及每日归纳。截图/OCR 不是本人原话或真实关系证据，人物画像和关系增量仍校验对应人物、原消息及引文。

视觉观察兼容纯JSON、完整JSON代码围栏，以及兼容服务忽略json_object时返回的有界纯文字；纯文字封装为summary并标明非结构化、未独立核验。不完整围栏、混杂JSON、空白、超长或敏感内容仍拒收。失败日志只记录消息编号、图片序号与失败类别（图片/模型请求、超时或观察不合规），不记录图片链接、正文或Provider原始错误。

定向识图先下载并验证真实图片，再将有界 PNG 像素送入模型，避免兼容服务忽略 QQ 临时 URL 后凭空描述。图片过期、下载或解码失败时明确失败，不发送只有链接的识图请求。每张图片含下载/排队/模型调用最多40秒，一轮最多3张，总计最多125秒。

记忆库管理页复用当前面板组件与配色，默认按头像、昵称和 QQ 展示一人一份当前画像，页头统计人物数量；进入人物先展示其综合资料，五类字段归在同一份记录内，来源依据默认折叠。左侧支持按昵称、QQ、群名片、画像及历史内容搜索，不按群复制人物。历史记忆及每日摘要在选择人物后主动打开才读取，原有大量历史材料不作为多份画像展示。会话仅筛选历史来源；全局画像综合各群和私聊中通过本人原话校验的当前字段，同字段由较新的有效原话更新，迟到旧资料不覆盖。群自身的记忆保留会话归属，不按群成员批量分配给个人。头像缺失时显示名字首字，未获取群名时显示会话类型和编号。

`GET /api/memories`、`GET /api/summaries` 保留 `owner_type/owner_id/limit`，增加 `person_id/chat_id/chat_type/search/page/page_size`；分页每页默认24、最多96条，响应包含真实筛选总数和记忆/摘要计数，避免把截取的200条当全库数量。空内容、字面null/undefined不展示，原记录保留。列表展示消息记忆、窗口归纳、历史记忆等来源标签，旧记录没有出处时明确缺失。列表只读，不改变 Bot 记忆检索或跨会话使用规则。

新增受管理员认证保护的 `GET /api/memories/people`（统一人物，分页/搜索）和 `GET /api/memories/conversations`（可按person_id列出参与的会话，最多200个）。`GET /api/memories/{id}/source?kind=memory|summary` 按需读取已保存出处的至多60条消息，限制会话、类型、消息区间；显式个人记忆额外验证作者。来源不明、已丢失或不匹配时返回空，不从邻近消息猜测。已有个人每日摘要缺少会话字段，保留全局人物归属，不能据此回填来源或匹配某个群筛选。图片URL不返回；敏感正文隐藏，长消息限4000字。

人物画像沿用现有本人原话与字段校验，独立于来源筛选显示；点击关系图谱通过 `person` 查询参数定位同一个人物。页面快速切换、筛选、翻页及离开时取消旧请求，出处抽屉只在打开时加载；错误提供重试，界面不加入原理解释或示意文案。

`GET /api/memories/people` 返回真实人物总数、当前 `profile`、`profile_updated_at` 和已关联的来源 `aliases`。确认关联的 OneBot 与 QQ 官方身份在搜索、会话筛选和分页前归并成一张卡，已有 OneBot 人物时沿用其卡片 ID。相同昵称的未关联人物仍独立显示。最多96张卡的画像在一次批量查询内读取，同字段采用较新的有效原话；后台原始发言、画像历史、活跃统计、记忆和摘要可按统一人物查看，出处保留原作者和会话。显式整理优先处理关联身份中最新的待整理发言，每次仍按一个原始身份执行。仅浏览页面不会调用模型。

全局画像目录保留“历史记录”次级入口，用于按会话浏览群自身的记忆与成员历史，返回画像页不继续展示历史明细；历史搜索保留在历史视图。默认入口仍为一人一份画像，不自动读取这些明细。

历史视图中点击侧栏的同一人物或“全部画像”，也会返回对应画像，不必先切换到其他人物。

人物画像还支持 `interests`（兴趣爱好）。画像整理按同一人物的未处理原始文本组合近期与历史材料（群聊可合并，私聊仅同一会话，不混入公开批次），每批最多160条，单条最多4000字、原文预算最多32000字；超长正文明确截断，完整原文保留。敏感原文跳过但推进已检查范围；预算用完前未读取的消息不推进游标。每字段须绑定本批本人消息和逐字引文，较新的有效来源更新当前字段；无新依据不清空画像，不从日记递归总结人物。

自动画像整理与群归纳共用夜间调度，默认北京时间03:00开始，先整理会话的新事实和日记，再处理人物队列。人物每晚最多10人，完成一人后等待60秒；默认前8个人优先本人新事实线索及近期发言，后2个按最近成功整理时间轮转（其他人数额度按约20%保留轮转位）；失败尝试不冒充成功，同一人物当晚不重复领取。调度、日额度、人物占位及间隔沿用现有持久机制，失败也占额度；03:00–06:00恢复窗口外不补跑画像。`portrait_daily_limit`（默认10，0–100）、`portrait_interval_secs`（默认60，60–3600）仍在运行参数热配置，`run_on_startup` 仅立即运行会话归纳。会话另有 `conversation_daily_limit` 默认20个模型批次（0–200），失败也计入当天预算，不占人物额度；超过预算的消息保留待处理。调度按顺序运行会话和人物，画像的夜间/手动入口共用串行锁，模型等待时不占数据库事务。

V11将原始消息、带出处事实、会话日记、画像当前投影和版本历史分开保存，借鉴[腾讯开源记忆分层](https://github.com/TencentCloud/TencentDB-Agent-Memory)；本地实现及取舍见[ADR-0011](../../adr/0011-incremental-memory-storage.md)。这不代表接入了腾讯服务或向量检索。

- `messages` / `message_images` 保留原始文本与图片观察，不删旧消息。`memory_facts` 保存本批提炼的事实、人物或会话归属、原消息编号、本人引文、会话类型/编号及来源时间；重复的同内容同原文事实不重复写入。个人事实必须来自该人原话；群事实保留实际消息来源，不把窗口摘要当单条证据。旧 `long_memories` 全部保留，新事实在同一事务投影进去供现有召回接口使用。
- `memory_diary_entries` 按批次、人物/会话及消息实际北京时间日期追加，记录完整来源区间和批次发言数。群聊和同编号私聊分开；同一人物来自多个群的日记互不覆盖。没有可用个人归纳时不拿发言计数冒充摘要。旧 `summaries` 原样保存，与新日记一起浏览；新日记在现有API使用负数ID，旧记录仍用原正数ID。
- `person_profile_facts` 仍是一人一份当前画像，插入或有效来源更新时由SQLite触发器原子保留 `person_profile_versions`，包括更正与再次确认。普通快照从有效历史中读取不晚于消息上界的版本，避免当前画像更新后旧窗口拿不到当时资料；迁移前已经丢失的旧版本不能恢复。
- `memory_progress` / `memory_batches` 按处理阶段、人物/会话编号和类型保存连续游标、批次边界、租约与完成/失败状态。结果与游标同事务提交，模型失败不消费未处理消息，迟到或已失效批次整体回滚。会话每次最多500条/32000字原文，同一批只跨一个实际日期；后续仍有消息时保留待处理。会话模型最多等待180秒，画像沿用夜间180秒预算。
- 熟悉度的纯SQL @统计使用独立游标，不因模型失败而重复计数，也不依赖模型可用。新事实保留兼容召回，Bot按当前会话类型筛选已知来源，避免群聊/私聊同编号串用；旧无出处材料仍标记未知，不补造来源或宣称已验证语义真实性。

升级时不重新付费归纳全部旧聊天：旧会话消息上界及人物已有整理进度记为显式 `baseline`，旧原文、事实、摘要、画像及配置原样保留。基线表示兼容边界，不表示旧消息曾逐条经过新处理器；没有整理进度的人仍按夜间限额补整理。既有熟悉度作为升级基线保留，不重新累计旧@；新增会话消息从边界后连续处理，人物使用下述精确覆盖。迁移把现存画像留作初始版本，原整理时间未知时记0，不能补造更早的变化历史。重放迁移保留已有进度，不重置游标；画像单项校验失败跳过，数据库写入失败则整体回滚该批结果和进度。

人物页“画像变更”按需读取受认证保护的 `GET /api/memories/people/{id}/profile-history`，分页默认24、最多96；显示当前/历史值、来源会话、原话时间和依据，`GET /api/memories/{version_id}/source?kind=profile` 可查看校验过的原消息。切换人物、关闭抽屉或离开页面取消旧请求。旧来源无效时保留历史记录，不能当作已核验当前画像。列表及原文隐藏敏感内容，不返回图片临时地址。实现验收以本提交mock/SQL/API/浏览器测试为准，尚未代表生产提炼与部署成功。

人物详情新增“原始发言”：管理员认证保护的 `GET /api/memories/people/{id}/messages` 直接读取该人物跨群与私聊的原消息，按真实时间倒序分页（默认24、最多96），支持 `date=YYYY-MM-DD`、`utc_offset_minutes`（默认480，范围−840至840）、会话类型/编号及正文搜索。日期转换成明确的UTC半开区间，独立于摘要日期与服务器时区；没有日期时读取全部日期。图片仅标记存在，不返回临时图片地址；敏感正文隐藏，长消息截取前4000字并标记，未知会话类型不展示。

`GET /api/memories/people/{id}/activity` 返回按查看者时区计算的真实今日条数、最近发言、画像最近整理状态/时间、准确待整理条数（含近30天）、已有处理条数、旧基线条数及最近批次诊断。浏览只读，不调用模型；画像事实更新时间与最近整理时间分开表达，空提炼和失败不会伪装成画像改变。选择人物后读取统计，原始发言仍按需读取；可见页面每分钟刷新当前人物状态及画像/发言第一页，切换人物或离开时取消请求。旧个人摘要的“今日发言N条”实际上是归纳窗口计数，界面将旧计数统一标为“归纳窗口内发言”，新日记不再写入这种计数兜底，保留原历史记录，不用它充当今日统计。

人物页提供“整理画像”，显式调用受 session 保护的 `POST /api/memories/people/{id}/refresh`；无人物返回404，未配置 Decision 返回503，模型/数据库失败保留当前画像并提示重试。整理使用现有 Decision 网关的限速、并发与超时，每次正常一次模型调用，格式不合规仅允许一次修复，两次共用180秒总期限，同人物通过数据库原子领取限制5分钟内重复调用；进度及更新时间保存在现有 `state_kv`，增量游标没有新有效消息时不重复提炼。空结果保留空缺，不补造性格。读取/浏览接口不触发付费调用；仍保留历史记录页。该功能由 mock 与浏览器测试验证，不代表已部署或真实模型语义验收。

## 边界

- 归纳结果影响后续快照，不 retroactively 修改当前回答；摘要不等于长期人格事实。
- 术语与语义约束参见[Bot 上下文](../../reference/bot-context.md)和[CONTEXT](../../../CONTEXT.md)。

## 对应实现（本页负责的主源码）

- [`crates/core/src/consolidation.rs`](../../../crates/core/src/consolidation.rs)
- [`crates/core/src/context_builder.rs`](../../../crates/core/src/context_builder.rs)
- [`crates/core/src/vision.rs`](../../../crates/core/src/vision.rs)
- [`crates/core/src/memory.rs`](../../../crates/core/src/memory.rs)
- [`crates/core/src/memory/portrait_queue.rs`](../../../crates/core/src/memory/portrait_queue.rs)
- [`crates/core/src/memory/refresh.rs`](../../../crates/core/src/memory/refresh.rs)
- [`crates/core/src/memory/nightly.rs`](../../../crates/core/src/memory/nightly.rs)
- [`crates/core/src/memory/processing.rs`](../../../crates/core/src/memory/processing.rs)
- [`crates/webui/src/memories.rs`](../../../crates/webui/src/memories.rs)
- [`crates/webui/src/memories/activity.rs`](../../../crates/webui/src/memories/activity.rs)
- [`webui-frontend/src/views/Memories.vue`](../../../webui-frontend/src/views/Memories.vue)

## 验证入口

- [`crates/core/tests/visual_context.rs`](../../../crates/core/tests/visual_context.rs)
- [`crates/webui/tests/memories_api.rs`](../../../crates/webui/tests/memories_api.rs)
- [`webui-frontend/tests/memories-flow.spec.js`](../../../webui-frontend/tests/memories-flow.spec.js)

- [`crates/core/tests/profile_refresh.rs`](../../../crates/core/tests/profile_refresh.rs)
- [`crates/core/tests/memory_storage.rs`](../../../crates/core/tests/memory_storage.rs)
- [`crates/core/tests/bot_context.rs`](../../../crates/core/tests/bot_context.rs)
- [`crates/core/tests/consolidation.rs`](../../../crates/core/tests/consolidation.rs)

维护规则：修改上列源码的功能行为时，同一 PR 更新本页；新增、删除或迁移模块时同步更新 `catalog.json`。测试入口只代表可追踪的验证位置，不代表本次执行通过。

## V12 上下文与语义记忆

新增 `memory_items` 追加保存有依据的事实、可修正归纳、事件和综合画像，带稳定主题键、来源会话、依据列表、有效期及 current/ended/uncertain/corrected 状态。旧 messages、long_memories、日记、画像和游标均保留，不回填虚构历史。归纳项至少三个不同时间的本人消息，图片观察和 Bot 发言不能充当本人证据；出处校验不能证明模型语义正确。人物/群条目分别接入现有画像队列和会话整理，复用模型调用；03:00、10人、60秒串行规则不变。

回复固定读取当前发言者、被引用者和真实@对象的可见短画像及当前群画像；群中不会读取私聊来源画像。跨群只共享公开来源的综合认识，事件详情限定来源会话，兼容长期记忆召回也过滤会话来源。只有 Router 提供 history_query 才查询当前会话近30天原文，最多80个候选、返回6条，包含真实 Bot 回答及作者/时间/引用信息；这是有界词项检索，不能保证语义召回完整。

回复锚点/上界继续固定；Chat 可提交结构化 execution_request，由 Decision 重新验证授权后派工。上下文稳定资料先序列化，动态事件后置，技能目录稳定排序；情绪移入动态资料。请求按UTF-8字节保守估算并预留8192输出空间，300k为硬上限，普通回复仍沿用小预算；不宣称精确 tokenizer 或达到300k有效输入。画像来源与版本仍由SQLite读取，共享快照避免同轮重复捕获；新增有界材料缓存（128项、60秒），键包含数据库、人物、会话可见范围、快照有效版本及SQLite失效世代；画像/记忆更新、原消息更改或删除通过触发器使缓存失效，有效期在每次读取时重新判断。执行历史阶段压缩见任务文档。

人物页附加简短综合认识、状态、来源会话和可展开原话依据；原始发言及旧画像/群历史继续保留。验证入口增加 [`context_runtime.rs`](../../../crates/core/tests/context_runtime.rs)，新增实现 [`semantic.rs`](../../../crates/core/src/memory/semantic.rs)。真实模型提炼质量与QQ回聊需部署后验收。

画像整理每次只处理连续前缀中同一可见范围的消息，遇到群/私聊切换或不同私聊即保留后续游标待处理，避免私聊内容在模型归纳阶段混入公开画像。公开资料仍按统一人物身份综合。夜间优先长期未整理者，再优先已有新事实线索者，其后按上次整理时间轮转。新增缓存实现 [`material_cache.rs`](../../../crates/core/src/memory/material_cache.rs)。

来源范围回归覆盖：公开的综合认识可跨群使用，事件详情只在原会话读取；私聊中的人物视图不会自动取得另一群的事件细节。

OneBot 与 QQ 官方机器人写入同一套人物、原文、归纳及记忆表。管理员关联已确认属于同一人的 OpenID 和 QQ 号后，对话可双向读取两侧现存和新增的、带真实群聊原文佐证的人物事实与画像。关联只影响记忆读取，保留原作者和来源校验，移除配置并重启即可撤销。私聊记忆、会话摘要、事件详情及任务权限仍按原会话或身份使用；会话与群号不会从昵称或 OpenID 推断关联。配置入口见 [QQ 接入](../adapter-qq/qq.md)。

上下文裁剪回归使用当前会话真实来源记录，验证被裁掉的消息不能继续充当Decision参与依据；无出处旧记录仍保留存储，不作为有来源会话召回的替代。

识图与近期话题保留近期实际参与者之间的有界互动边，帮助理解讨论；这不扩大固定人物画像加载范围，也不把互动指标当作现实关系证据。

人物目录批量读取综合画像预览，并纳入画像搜索与更新时间；只有综合画像、没有旧五类字段的人也显示已有画像。API回归覆盖此情况。

接话快照增加 `scene.activity` 与 `activity_as_of_ts`，按固定上界末条消息时间计算同群人类热度，周期批次不以最早消息时间误算，也不混入后到消息、其他Bot或同号私聊。`dialogue.group_recent_replies`至多保留近5分钟两条Bot实际发言（各180字）及真实原anchor（各120字），标明回复对象；理解群聊共同话题时不改变同一人的last_reply_to_sender归属，Bot旧回答仍不是事实证据。Chat预算不足先裁掉可选群内衔接，不挤掉锚点和引用。回归位于既有 `bot_context.rs`；参与阈值见[决策功能](decision.md)，不修改30天检索与长期记忆存放。

快照增加bot_identity（self及SQLite身份表中的QQ别名，稳定资料段）和addressing（协议@/私聊确认的明确对象、引用作者及消息号）。Decision与Chat共享这些字段，原始mentions与引用作者不改写；明确@Bot不被引用作者覆盖，多人同时被@不强制选Bot。群聊周期输入仅复用Bot身份，实际锚点仍按批次选择。bot_context.rs覆盖“引用群友原文＋@云团改写”、同群他人对话、多人@和私聊，并验证读取不改原始记录。

快照增加当前可见表情类别的self/other归属和实际发送统计，Decision与Chat共享meme_library。预算裁剪时派生图包清单随meme_categories同步收缩，避免目录回补已裁剪类别。成功发出的库存图保留类别、图包与真实回复锚点供短时承接；未送达或旧未知图片不虚构身份。数据仍在原messages表中，历史统计按chat_id/chat_type、固定消息上界和时间过滤，不更改人物事实或群聊长期记忆。

## 图片引用、复用与纠正（V15）

Decision输入的`images`和Chat的`visual_context`共享固定快照，图片条目带`source_msg_id/image_index/shared_by/reference_kind/representation`。附图、显式引用优先；没有引用时，最多关联180秒内最近图片，且须是同一发言者上一条发言的图片，或云团刚回复给此人的库存图。期间出现其他人的图片、引用指向别的文字或超时后不猜对象。`recent_context`只表示邻近资料，不能当作本轮图片对象。普通群聊周期判断读取已缓存描述，不额外批量收费识图。

首次需要读图时仍由bot_chat视觉入口处理真实像素，Decision无需切成多模态模型，也不新增一轮Decision。已识别条目直接使用文字；重复上传先在本地下载、解码、限尺寸并计算规范PNG的精确摘要，同一chat_id/chat_type、固定消息上界内有已完成记录时复用文字，不再次送像素给模型。摘要只作缓存键，不作身份或权限证明；不做近似图片混用，也不跨群/私聊复用私有观察和纠正。并发中的首次读取不保证合并。

Decision可输出至多3项`image_corrections:{source_msg_id,image_index,description,evidence_quote}`。宿主核对当前原消息、作者、逐字引文、可见图片、会话与上界，明确指向其他成员、ignore或执行动作不写纠正。纠正按图片和纠正消息追加保存，重复处理幂等，不覆盖原始视觉观察或原文。当前回复前刷新文字描述，后续同图优先显示有效的最新纠正及其作者/原话；这是用户陈述，`verified_by_pixels=false`，不宣称独立验证，也不据此更新真实身份或人物关系。

来源、文字缓存和纠正随SQLite备份保存。图库成功发送且QQ有消息回执时，在同一事务保存self消息与meme_id；之后可通过受图库根目录约束的本地文件回看实际图像，失败发送和旧未知图片不补造。临时生图附件仍沿用原清理策略，未扩展为永久图片仓库。

实现新增[`vision/cache.rs`](../../../crates/core/src/vision/cache.rs)，回归新增[`visual_followup.rs`](../../../crates/core/tests/visual_followup.rs)。本轮真实接口单次探测能描述耳罩与猫耳位置，但返回纯文字；该探测未发送QQ消息，不能替代完整链路验收。

Linux CI在工作区全量回归前先执行visual_context与visual_followup专项，失败时停止后续发布编译。

## 已登记表情的精确来源

收到图片后，在原8MiB下载和解码限制内，先以文件摘要筛选active、added_by=admin、属于云团专属标记的图库候选，再核对图库根内当前文件的完整字节。只有字节完全一致才绑定message_images.meme_id并保存“登记为云团虚构角色表情”的文字；摘要碰撞、近似外观、篡改文件、pending或群聊自动采集均不能产生登记命中。元信息与分享者身份分开，real_person_identity/identity_confirmed不变为真实身份已确认。未知图片继续使用真实像素观察，不通过白发猫耳外观猜名字。

Context为实际发送来源或精确文件命中附加registered_asset，包含character_name、category、owner_person_id及来源；Decision与Chat可直接说明已登记虚构角色归属，不把“未确认现实身份”误解为不能认识自己的表情。这不是新做的画面细节分析；未描述的动作细节仍不能据登记推断。同会话纠正仍优先，公开登记可跨会话识别，私人纠正不跨会话复用。

图片观察失败的安全类别以vision:failure:<msg_id>:<image_index>键保存在state_kv，与观察状态同事务写入；只保存timeout、invalid_observation、image_or_provider，不含URL、正文或模型原错，成功后清除。context.failure可区分服务超时/返回不合规/读取或服务失败；历史未保存原因时保持未知，不补造“图片模糊”。

视觉请求锚点原文限1000字并标记截断，辅助人物/关系/引文按整体6000字符预算裁剪；仍保留原消息编号和固定图片来源。输出上限4096，普通聊天与Decision原输出预算不变；原40秒/图、3张/轮及总期限保持，不用追加重试或延长总等待掩盖问题。新实现[`vision/registered.rs`](../../../crates/core/src/vision/registered.rs)，回归[`visual_registered.rs`](../../../crates/core/tests/visual_registered.rs)覆盖已知图零视觉调用、陌生图不继承名字、来源与字节校验、会话隔离、纠正、预算及失败恢复。

这次生产记录核对表明引用正确且原图与图库128完全相同，但模型观察在40秒被取消；相同原图使用紧凑请求的真实接口探测6.3秒返回有效描述，未发送QQ消息。新宿主匹配与完整QQ回复仍按部署后实际行为验收，不以该探测代替发送成功。


## 补发@的待答上下文

[`crates/core/src/context_builder/conversation.rs`](../../../crates/core/src/context_builder/conversation.rs)在同一快照内补充`dialogue.attention_request.source`：仅关联当前作者在同一chat_id/chat_type、当前消息之前近5分钟的最近非空发言，且未被实际Bot发送记录回答。它与`anchor`分别保留真实消息编号、作者、时间和原文截断标记；`execution_authorization=false`，不会把旧指令拼进当前原话。明确面向其他人、过期或已有回答的发言不命中，后来的消息不能进入旧快照；参考保留在固定JSON字段，不依赖最近消息列表是否被裁剪。

未关联到本轮请求的群聊图片仅在已有观察时作为背景进入 `visual_context`。显式附件、引用图片和成功关联的追问继续保留识图状态。收到图片但未触发识图时，数据库中的 `pending` 表示等待识图；进入处理后失败才有对应的失败状态。文字引用附近的未处理图片不会被投影为本轮识图失败。

`dialogue.pending_requests`最多3条该作者尚处于deciding/executing的请求，正文最多160字。真实状态不等于成功凭据，不能据此编造“加载卡住”等原因。普通群聊仍每30秒判断增量，@仍走即时入口；不把30天原文全量塞进上下文。详见[会话调度](conversation.md)。新增`bot_context`与`dispatch_tests`回归覆盖归属、上界、在途请求和已答去重；真实模型选择与QQ送达需另行验收。


## 后续设计边界

后续工作记忆与任务过程记忆的职责、来源和即时更新目标集中在 [ADR-0018](../../adr/0018-lightweight-agent-runtime.md)。现有消息/任务记录继续保留；不要将设计接受误写为已完成工作记忆迁移。

## 即时事项工作记忆（V16）

新增 [`crates/core/src/working_context.rs`](../../../crates/core/src/working_context.rs)：人物画像之外保存同一作者、同一会话内的当前目标与条件。Router输出可选的working_memory及focus_context_id；Runtime逐字核对当前存储原话、作者、群/私聊归属和预期版本，再原子追加working_context_versions。新条件按key合并，纠正保留旧版本，重试同一来源不重复写入。目标和每项条件均带原消息编号，不能作为执行授权或完成凭据。

快照只读上界内、近30天更新的最近3个事项候选；Router按预算可移除较旧候选，Chat和Agent只接收本轮明确选中的事项。不回填历史推断；每轮最多12项、累计最多24项条件，每段原话2至400字、每事项原话合计最多4000字。超过限制拒绝本次更新并记录原因，不覆盖旧数据、不触发额外模型重试。工作记忆目前没有管理UI，也不会自动修改运行中Agent的任务范围；完整任务档案、统一工具协议和Docker浏览器仍属ADR-0018后续阶段。

回归入口：[`crates/core/tests/working_context.rs`](../../../crates/core/tests/working_context.rs)，覆盖条件延续、纠正版本、重复事件、归属、来源与快照上界；不代表真实模型提取已验收。

任务过程记忆补充V17独立任务说明，目标/条件/补充不再只依赖可压缩历史。快照增加同一作者和会话的有界task_context候选；Router选中后才为Chat展开说明及回执预览，已结束任务仍可追问，未选中的旧任务不主动接入本轮。人物与群聊记忆及夜间整理规则不变，详细来源和恢复边界见[任务功能](tasks.md)。


回复快照增加当前底层工具概览，渲染到capabilities.task_tools；tools_available综合Skill和实际Tool数量，不能因无Skill或概览被裁剪就否认工具能力。概览是可缩的临时辅助上下文，不写入人物画像、群记忆或原始消息，且优先于原话资料裁剪。具体内容与验证见[任务功能](tasks.md#工具发现与参数修正)。


## V19：近期增量、历史补齐与整理诊断

[`portrait_queue.rs`](../../../crates/core/src/memory/portrait_queue.rs)保存逐条消息覆盖：优先取近30天内最新的120条，再用同一批160条额度补最旧缺口；总原文仍限32000字符，有历史补齐时近期部分最多24000字符，余量留给旧材料。不对每个区间另开批次或增加当晚人数。来源按消息编号排序进入模型，敏感原文只标检查，不传给模型；被预算截断而未读的消息保持待处理。`portrait_coverage`与有效画像、结果状态同事务提交；连续游标只能推进到首个未读缺口之前，近期完成绝不把中间历史积压当作已读。返回的批次最小/最大编号和时间是边界，不能解释为区间全部消息都已处理。

本人明确的新信息继续复用Router现有`profile_updates`，校验本人原消息及逐字引文后即时更新并保留版本，夜间优先考虑尚未覆盖的事实线索；不新增逐消息模型调用。回填旧材料不能覆盖较新的字段或同主题综合记忆。群聊记忆、私聊范围、历史版本和原始消息继续保留。

会话每批、人物每次从共享`llm_slot`获取当前Decision网关，修复过程固定使用该批模型；保存模型配置后下一批生效，无需重建调度。`portrait_runs`关联`memory_batches`，记录实际Provider/模型/思考配置、来源时间边界、处理时间、耗时、格式修复次数、读取/写入/拒收数量和安全错误类别。模型HTTP失败、请求受限、JSON无效、依据全无效、超时、存储失败与中断分开；不保存原始模型答复、Provider错误正文、链接或凭据。非空提取全部未通过依据校验时整批失败，进度不推进；部分有效时仅写有效项目并记录拒收数。空提取是“已检查，暂无新资料”，不是画像更新。

连续3个人出现同模型配置、同类别失败时，持久保存当晚熔断，停止剩余人物领取；重启不清额度，同一天换模型配置可继续剩余额度。失败也计入当晚人数，每人仅一个批次，成功并不等同于画像有变化。中断的运行租约到期后可重新领取，未完成结果不得落库。人物页显示最近成功整理、待处理数量和按需展开的批次详情；原始发言时间与整理处理时间明确分开，旧故障没有原因时显示未知，不补造解释。

回归：[`profile_refresh.rs`](../../../crates/core/tests/profile_refresh.rs)、nightly单元测试、[`memories_api.rs`](../../../crates/webui/tests/memories_api.rs)、[`memories-flow.spec.js`](../../../webui-frontend/tests/memories-flow.spec.js)覆盖新旧不连续范围、字符额度、热换模型、有限修复/熔断、来源/私聊隔离、失败回滚、V18升级及UI。Mock和浏览器测试不代表真实夜间模型结果或服务器已经更新。

## V24：平台无关记忆身份接口

新增迁移 [`memory_identity_v24.sql`](../../../crates/core/src/db/memory_identity_v24.sql)，在核心层建立平台无关的 `memory_identity_links`、`memory_aliases` 与 `memory_directory`（自动回填既有 `qq_memory_links` 并通过触发器保持向后兼容）。核心记忆查询（[`memory.rs`](../../../crates/core/src/memory.rs)、[`semantic.rs`](../../../crates/core/src/memory/semantic.rs)）、上下文构建（[`context_builder.rs`](../../../crates/core/src/context_builder.rs) 自身多平台别名）与 WebUI `/memories`（[`memories.rs`](../../../crates/webui/src/memories.rs)、[`Memories.vue`](../../../webui-frontend/src/views/Memories.vue)）改为使用通用身份视图、由平台适配器注入 `AvatarResolver`（`adapter-qq` 提供 QQ 头像格式化），并支持按 `platform`（`onebot` / `qq_official`）筛选隔离不同平台记忆与会话列表。

`/memories` 入口路由改为先展示各接入平台的状态与统计卡片（经 `GET /api/memories/platforms` 返回各平台的人物、记忆、摘要及图谱节点/连线数），点击平台卡片进入 `/memories/:platform` 后按所选平台隔离展示原有记忆库详情页，并可跳转同平台的 `/relations/:platform?person=<id>`。

夜间与手动画像整理（[`nightly.rs`](../../../crates/core/src/memory/nightly.rs)、[`portrait_queue.rs`](../../../crates/core/src/memory/portrait_queue.rs)、[`refresh.rs`](../../../crates/core/src/memory/refresh.rs)）已按核心 `memory_directory` / `memory_aliases` 统一领取与批处理：同一关联人物在 OneBot 与 QQ 官方双侧发言时仅占用 1 个夜间名额与 1 次模型整理请求（共享一份 `current_profile` / `current_memory` 并合并公开群聊批处理，私聊仍保持单会话隔离）；跨关联通道在 120 秒内内容相同的重复原话只向模型发送一次并同步标记双侧覆盖游标，回写画像与记忆条目保留真实原作者 `sender_pid`（[`activity.rs`](../../../crates/webui/src/memories/activity.rs) 统计同步改用 `memory_aliases`）。
