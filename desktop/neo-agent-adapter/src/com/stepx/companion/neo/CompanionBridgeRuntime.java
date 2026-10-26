package com.stepx.companion.neo;

import android.app.KeyguardManager;
import android.content.Context;
import android.os.Handler;
import android.os.Looper;
import android.view.accessibility.AccessibilityNodeInfo;
import org.json.JSONArray;
import org.json.JSONObject;
import java.util.ArrayList;
import java.util.Collection;
import java.util.concurrent.FutureTask;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.TimeUnit;

/** Original IPC orchestration. Planning and execution stay in the Android process. */
final class CompanionBridgeRuntime {
    private static volatile CompanionBridgeRuntime instance;
    static synchronized CompanionBridgeRuntime get(Context context) {
        if (instance == null) instance = new CompanionBridgeRuntime(context.getApplicationContext());
        return instance;
    }
    static void phoneTakeover() {
        CompanionBridgeRuntime live = instance;
        if (live == null) return;
        // Never wait on the runtime monitor from the Android main thread: a
        // commit may itself be awaiting a main-thread launch callback.
        live.phoneEpoch.incrementAndGet();
        ModelClient current = live.planner;
        if (current != null) current.cancel();
        live.monitor.execute(() -> {
            synchronized (live) {
                if (live.record.optString("state").equals("planning")) {
                    ++live.epoch;
                    try { live.record.put("state", "paused_planning").put("detail", "你已在手机接管，旧规划不会启动操作"); live.persist(); }
                    catch (Exception ignored) {}
                }
            }
        });
    }
    private final Context context;
    private final Handler main = new Handler(Looper.getMainLooper());
    private final ScheduledExecutorService monitor = Executors.newSingleThreadScheduledExecutor();
    private JSONObject record = new JSONObject();
    private volatile ModelClient planner;
    private final java.util.concurrent.atomic.AtomicLong phoneEpoch = new java.util.concurrent.atomic.AtomicLong();
    private long epoch;
    private String configNote = "";
    private CompanionBridgeRuntime(Context context) {
        this.context = context;
        try {
            String stored = new Vault(context).get("stepx-bridge-task-v1");
            if (!stored.isEmpty()) record = new JSONObject(stored);
            if ("planning".equals(record.optString("state"))) {
                record.put("state", "interrupted").put("detail", "规划进程已重启，请重新发起任务");
                persist();
            }
        } catch (Exception ignored) { record = new JSONObject(); }
        scheduleMonitor();
    }
    private void scheduleMonitor() {
        long delayMs;
        synchronized (this) {
            String st = record.optString("state");
            boolean activeTask = !st.isEmpty() && !BridgePolicy.terminal(st);
            delayMs = activeTask ? 1000L : 10000L;
        }
        monitor.schedule(this::monitorTick, delayMs, TimeUnit.MILLISECONDS);
    }
    private void monitorTick() {
        synchronized (this) {
            try { refresh(); } catch (Exception ignored) {}
        }
        scheduleMonitor();
    }
    private interface MainCall<T> { T run() throws Exception; }
    private <T> T onMain(MainCall<T> action) throws Exception {
        if (Looper.myLooper() == Looper.getMainLooper()) return action.run();
        FutureTask<T> future = new FutureTask<>(() -> action.run());
        main.post(future);
        try { return future.get(7, TimeUnit.SECONDS); }
        catch (Exception failure) { future.cancel(false); throw failure; }
    }
    synchronized JSONObject request(JSONObject q) {
        try {
            switch (q.optString("op")) {
                case "host_ui": CompanionHostSurface.update(q.optBoolean("visible"));return new JSONObject().put("ok",true);
                case "status": return status();
                case "config": return configure(q);
                case "config_info": return publicConfig();
                case "apps": return applications(q);
                case "start": return start(q);
                case "pause": return control(q, "pause");
                case "resume": return control(q, "resume");
                case "stop": return control(q, "stop");
                default: return CompanionBridgeProvider.failure("unknown_operation", "不支持的操作");
            }
        } catch (BridgeFailure failure) {
            return CompanionBridgeProvider.failure(failure.code, failure.getMessage());
        } catch (Exception failure) {
            return CompanionBridgeProvider.failure("operation_failed", "操作未完成，请检查模型配置、应用授权和无障碍状态");
        }
    }
    private void require(boolean value, String code, String message) throws BridgeFailure {
        if (!value) throw new BridgeFailure(code, message);
    }
    private boolean busy() {
        ControlService service = ControlService.instance;
        return record.optString("state").equals("planning") || (service != null && service.busy());
    }
    private JSONObject configure(JSONObject q) throws Exception {
        if (!q.has("endpoint") && !q.has("model") && !q.has("key")) return publicConfig();
        require(!busy(), "busy", "请先停止当前任务，再修改模型");
        require(q.optBoolean("userConfirmed"), "confirmation_required", "请在伴侣中确认模型接收端和截图授权范围");
        Config previous = Config.load(context), c = new Config();
        c.endpoint = q.optString("endpoint", previous.endpoint).trim().replaceAll("/+$", "");
        c.model = q.optString("model", previous.model).trim();
        c.protocol = BridgePolicy.protocol(q.optString("protocol", previous.protocol));
        require(c.protocol != null, "protocol", "请选择 OpenAI、Anthropic 或 Gemini 协议");
        c.key = q.optString("key", "").trim();
        if (c.key.isEmpty() && previous.endpoint.equals(c.endpoint) && previous.protocol.equals(c.protocol)) c.key = previous.key;
        require(c.key.length() <= 16384 && c.model.length() <= 256 && !c.model.matches("(?s).*\\p{Cntrl}.*")
            && !c.key.matches("(?s).*\\p{Cntrl}.*"), "config_invalid", "模型配置格式无效");
        c.vision = true;
        c.maxSteps = Math.max(4, Math.min(40, q.optInt("maxSteps", previous.maxSteps)));
        c.checkout = false;
        c.save(context);
        configNote = "模型设置已保存到 Android Keystore 加密存储";
        return publicConfig();
    }
    private JSONObject publicConfig() throws Exception {
        Config c = Config.load(context);
        return c.json().put("ok", true).put("hasKey", !c.key.isEmpty()).put("note", configNote)
            .put("scopeMode", context.getSharedPreferences("companion-scope",0).getString("mode","default"));
    }
    private JSONObject applications(JSONObject q) throws Exception {
        AppAccess access = new AppAccess(context);
        if (q.has("packages") || q.has("mode")) {
            require(!busy(), "busy", "请先停止任务，再修改可操作的应用");
            require(q.optBoolean("userConfirmed"), "confirmation_required", "请先确认允许操作的应用与截图接收端");
            String mode=q.optString("mode","custom");
            require(mode.equals("all")||mode.equals("custom"),"scope_mode","应用范围模式无效");
            JSONArray packages = mode.equals("all")?installedPackages(access):q.getJSONArray("packages"), choices = new JSONArray();
            require(packages.length() <= 512, "scope_limit", "应用范围过大");
            for (int i = 0; i < packages.length(); i++) {
                String pkg = packages.getString(i);
                require(pkg.matches("[A-Za-z0-9_]+(?:\\.[A-Za-z0-9_]+)+"), "package", "应用包名无效");
                choices.put(new JSONObject().put("package", pkg).put("enabled", true));
            }
            access.save(new JSONObject().put("apps", choices));
            Config c = Config.load(context);
            if (!access.approved().isEmpty()) new RoutineAccess(context).approve(c, access.approved());
            require(context.getSharedPreferences("companion-scope",0).edit().putString("mode",mode).putString("endpoint",c.endpoint).commit(),"storage","未能保存应用范围模式");
        }
        JSONObject state = access.state();
        JSONArray apps = state.getJSONArray("apps");
        for (int i = 0; i < apps.length(); i++) apps.getJSONObject(i).remove("icon");
        return state.put("ok", true).put("mode",context.getSharedPreferences("companion-scope",0).getString("mode","default"));
    }
    private JSONArray installedPackages(AppAccess access)throws Exception{
        AppAccess.invalidate();JSONArray installed=access.installed(),packages=new JSONArray();
        for(int i=0;i<installed.length();i++)packages.put(installed.getJSONObject(i).getString("package"));
        return packages;
    }
    private void refreshFullScope(Config c)throws Exception{
        android.content.SharedPreferences prefs=context.getSharedPreferences("companion-scope",0);
        if(!"all".equals(prefs.getString("mode","")))return;
        require(c.endpoint.equals(prefs.getString("endpoint","")),"scope_consent","模型接收端已变化，请在模型设置保存确认");
        AppAccess access=new AppAccess(context);JSONArray packages=installedPackages(access);
        java.util.Set<String> present=new java.util.HashSet<>();for(int i=0;i<packages.length();i++)present.add(packages.getString(i));
        if(!present.equals(new java.util.HashSet<>(access.approved()))||!new RoutineAccess(context).allows(c,present))
            applications(new JSONObject().put("mode","all").put("userConfirmed",true));
    }
    private JSONObject start(JSONObject q) throws Exception {
        String id = q.optString("requestId"), goal = q.optString("goal").trim();
        require(BridgePolicy.requestId(id), "request_id", "需要有效的 requestId");
        if (id.equals(record.optString("requestId"))) {
            require(goal.equals(record.optString("goal")), "request_conflict", "同一 requestId 不能对应不同目标");
            return status().put("idempotent", true);
        }
        require(!busy(), "busy", "另一个任务仍在执行或等待接管，请先结束该任务");
        require(!goal.isEmpty() && goal.length() <= 6000 && !PrivateText.credential(goal), "goal", "请输入不含密钥的有效任务目标");
        ControlService service = ControlService.instance;
        require(service != null && service.connected, "accessibility", "请在手机启用 Neo Agent Touch 无障碍服务");
        require(!((KeyguardManager) context.getSystemService(Context.KEYGUARD_SERVICE)).isKeyguardLocked(), "locked", "请先手动解锁手机");
        Config c = Config.load(context); c.validate();
        refreshFullScope(c);
        AppAccess access = new AppAccess(context);
        require(!access.approved().isEmpty(), "applications", "请先确认允许操作的应用");
        JSONObject verify = q.optJSONObject("verify");
        if (verify != null) {
            require(verify.optString("preset").isEmpty() || ("android_version".equals(verify.optString("preset")) && "打开设置，查看 Android 版本".equals(goal.trim()) && "com.android.settings".equals(verify.optString("package"))), "verification", "预设验收与原目标不一致");
            require(access.allowed(verify.optString("package")), "verification_scope", "验收应用必须属于已授权范围");
            JSONArray texts = verify.getJSONArray("texts");
            require(texts.length() > 0 && texts.length() <= 6, "verification", "验收需要 1 到 6 项界面文字");
            for (int i = 0; i < texts.length(); i++) require(!texts.getString(i).trim().isEmpty() && texts.getString(i).length() <= 100, "verification", "验收文字无效");
        }
        record = new JSONObject().put("requestId", id).put("goal", goal).put("session", q.optString("session", "companion"))
            .put("state", "planning").put("detail", "正在用独立模型规划任务").put("createdAt", System.currentTimeMillis())
            .put("verify", verify == null ? JSONObject.NULL : new JSONObject(verify.toString())).put("runId", "");
        persist();
        beginPlanning(c);
        return status().put("accepted", true);
    }
    private void beginPlanning(Config c) throws Exception {
        final long token = ++epoch, phoneToken = phoneEpoch.get();
        final String goal = record.getString("goal"), owner = "stepx:" + record.getString("requestId");
        final ModelClient client = new ModelClient(); planner = client;
        record.put("state", "planning").put("detail", "正在规划"); persist();
        new Thread(() -> {
            try {
                String contextText = "user: " + PrivateText.redact(goal) + "\n" + new AppAccess(context).routing(goal);
                JSONObject plan = Conversation.plan(client, c, contextText);
                TaskWorkflow workflow = null;
                if (plan.optString("kind").equals("task")) {
                    try { workflow = new TaskWorkflow(context, plan, PrivateText.redact(goal)); }
                    catch (WorkflowPlanException invalid) {
                        plan = Conversation.plan(client, c, contextText + "\n计划校验未通过：" + invalid.safeFeedback() + "。仅按相同原话修正一次，不增加目标或权限。");
                        workflow = new TaskWorkflow(context, plan, PrivateText.redact(goal));
                    }
                }
                synchronized (this) {
                    if (token != epoch || phoneToken != phoneEpoch.get() || !record.optString("state").equals("planning")) return;
                    if (workflow == null) {
                        record.put("state", "needs_input").put("detail", PrivateText.redact(plan.optString("reply", "需要补充目标")));
                        persist(); return;
                    }
                    require(new RoutineAccess(context).allows(c, workflow.packages()), "scope_consent", "计划涉及未确认的应用或截图接收端，尚未执行");
                    JSONObject verification = record.optJSONObject("verify");
                    require(verification == null || workflow.packages().contains(verification.optString("package")), "verification_scope", "计划与验收应用不一致，尚未执行");
                    workflow.baseSteps = c.maxSteps;
                    final TaskWorkflow prepared = workflow;
                    ControlService service = ControlService.instance;
                    require(service != null && service.connected, "accessibility", "无障碍服务已断开，尚未执行");
                    boolean started = onMain(() -> {
                        if (phoneToken != phoneEpoch.get()) return false;
                        synchronized (service) {
                            if (service.busy()) return false;
                            CompanionCompletion.begin(service, owner, verification);
                            return service.startWorkflow(c, prepared, owner);
                        }
                    });
                    require(started, "not_started", "执行器未接收任务，请查看手机控制状态");
                    record.put("runId", service.runId).put("state", "running").put("detail", "手机执行器已开始，等待实际执行结果");
                    persist();
                }
            } catch (Exception failure) {
                synchronized (this) {
                    if (token != epoch || phoneToken != phoneEpoch.get()) return;
                    try { record.put("state", "failed").put("detail", failure instanceof BridgeFailure ? failure.getMessage() : ControlService.safeError(failure)); persist(); }
                    catch (Exception ignored) {}
                }
            }
        }, "Neo-companion-planner").start();
    }
    private JSONObject control(JSONObject q, String op) throws Exception {
        require(q.optString("requestId").equals(record.optString("requestId")) && !record.optString("requestId").isEmpty(), "stale_request", "任务编号已变化，未操作其他任务");
        if (op.equals("resume")) require(q.optBoolean("userConfirmed"), "confirmation_required", "继续任务需要用户明确交还控制");
        String state = record.optString("state");
        if (state.equals("planning") || state.equals("paused_planning")) {
            if (op.equals("resume") && state.equals("paused_planning")) beginPlanning(Config.load(context));
            else if (!op.equals("resume")) {
                ++epoch; if (planner != null) planner.cancel();
                record.put("state", op.equals("stop") ? "stopped" : "paused_planning").put("detail", op.equals("stop") ? "任务已停止" : "规划已暂停，尚未操作手机");
                persist();
            }
            return status();
        }
        ControlService service = ControlService.instance;
        require(service != null && owned(service), "task_unavailable", "原任务执行器不可用，未操作其他任务");
        if (op.equals("stop")) {
            ++epoch; onMain(() -> { service.stop("伴侣已停止本次任务"); return true; });
            record.put("state", "stopped").put("detail", "任务已停止"); persist();
        } else if (op.equals("pause")) {
            onMain(() -> { service.pauseForUser(); return true; }); refresh();
        } else {
            boolean resumed = onMain(service::resumeFromUser);
            require(resumed, "resume_rejected", "原任务不能继续，请在手机核对接管、屏幕锁定或执行上限");
            refresh();
        }
        return status();
    }
    private boolean owned(ControlService s) {
        return !record.optString("requestId").isEmpty() && ("stepx:" + record.optString("requestId")).equals(s.ownerSession)
            && record.optString("runId").equals(s.runId);
    }
    private void refresh() throws Exception {
        String old = record.optString("state");
        if (old.isEmpty() || old.equals("planning") || old.equals("paused_planning") || BridgePolicy.terminal(old)) return;
        ControlService s = ControlService.instance;
        if (s == null || !s.connected) {
            record.put("state", "paused").put("detail", "无障碍服务暂时断开，等待手机恢复"); return;
        }
        if (!owned(s)) {
            record.put("state", "interrupted").put("detail", "原任务已不拥有执行器，未操作当前任务"); persist(); return;
        }
        String next; JSONObject workflow;
        synchronized (s) {
            workflow = s.workflow == null ? new JSONObject() : s.workflow.json();
            next = s.running ? "running" : s.paused ? "paused" : "reported";
            record.put("detail", s.outcomeDetail.isEmpty() ? s.status : s.outcomeDetail)
                .put("step", s.nextStep).put("modelCalls", s.modelCalls).put("screenshots", s.screenshots).put("actions", s.actions)
                .put("workflow", workflow);
        }
        if (next.equals("reported")) {
            JSONArray steps = workflow.optJSONArray("tasks");
            String[] phases = new String[steps == null ? 0 : steps.length()];
            boolean[] verified = new boolean[phases.length];
            if (steps != null) for (int i = 0; i < steps.length(); i++) {
                JSONObject step = steps.getJSONObject(i);
                phases[i] = step.optString("state"); verified[i] = step.optBoolean("verified");
            }
            next = BridgePolicy.finishedState(phases, verified, false);
            if (next.equals("reported") && record.optJSONObject("verify") != null) {
                JSONObject proof;
                synchronized (s) { proof = owned(s) ? CompanionCompletion.finished(s) : new JSONObject().put("matched", false); }
                record.put("verification", proof);
                next = BridgePolicy.finishedState(phases, verified, proof.optBoolean("matched"));
            }
            record.put("goal_verified", next.equals("completed"));
        }
        record.put("state", next);
        if (!next.equals(old) || BridgePolicy.terminal(next)) persist();
    }
    private JSONObject status() throws Exception {
        refresh();
        ControlService s = ControlService.instance;
        JSONObject result = new JSONObject(record.toString());
        String state = result.optString("state", "idle");
        Config cfg = Config.load(context);
        boolean configComplete = false, scopeApproved = false;
        try {
            cfg.validate(); configComplete = true;
            java.util.Collection<String> approved = new AppAccess(context).approved();
            scopeApproved = !approved.isEmpty() && new RoutineAccess(context).allows(cfg, approved);
        } catch (Exception ignored) {}
        boolean connected = s != null && s.connected;
        // Never persist or reuse diagnostics from a replacement/native task.
        // Snapshot under the same lock used by ControlService task transitions.
        if (s != null) synchronized (s) {
            if (owned(s)) {
                String key = s.runConfig == null ? cfg.key : s.runConfig.key;
                result.put("diagnostics", new JSONObject()
                    .put("sampledAt", System.currentTimeMillis())
                    .put("trace", PrivateText.redact(BridgePolicy.diagnosticText(s.trace.toString(), key, 3000)))
                    .put("lastFailure", diagnosticFields(s.lastFailure, key, "stage", "code", "field", "types", "frames"))
                    .put("lastBoundary", diagnosticFields(s.lastBoundary, key, "terms", "checkout", "clickable", "class", "bounds", "own_text", "own_description", "label_hash"))
                    .put("input", CompanionInputDiagnostics.snapshot(s))
                    .put("modelResponse", CompanionModelDiagnostics.snapshot(s.model, s.startedAt))
                    .put("agentImeLease", diagnosticFields(s.agentImeLease.status(), key, "enabled", "active", "selected", "held", "restore_pending", "recovery_record_present", "opted_out_this_run", "restore_original_enabled", "state", "requires_manual_enable", "manual_choice_respected", "restore_verified")));
            }
        }
        return result.put("ok", true).put("engine", "Neo Agent Touch").put("adapterVersion", 1)
            .put("upstreamCommit", "a5a6b6aa28dece90831d0630214f514d23ef443e").put("state", state)
            .put("accessibilityConnected", connected).put("configComplete", configComplete).put("scopeApproved", scopeApproved)
            .put("ready", connected && configComplete && scopeApproved).put("terminal", BridgePolicy.terminal(state))
            .put("executorBusy", s != null && s.busy())
            .put("completed", state.equals("completed")).put("needsVerification", state.equals("reported"))
            .put("config", publicConfig());
    }
    private JSONObject diagnosticFields(JSONObject source, String key, String... fields) throws Exception {
        JSONObject result = new JSONObject();
        if (source == null) return result;
        for (String field : fields) {
            Object value = source.opt(field);
            if (value instanceof Boolean || value instanceof Number) result.put(field, value);
            else if (value instanceof String) result.put(field, PrivateText.redact(BridgePolicy.diagnosticText((String) value, key, 256)));
            else if (value instanceof JSONArray) {
                JSONArray list = new JSONArray(), original = (JSONArray) value;
                for (int i = 0; i < Math.min(original.length(), 20); i++)
                    if (original.opt(i) instanceof String) list.put(PrivateText.redact(BridgePolicy.diagnosticText(original.getString(i), key, 256)));
                result.put(field, list);
            }
        }
        return result;
    }
    private void persist() throws Exception {
        record.put("updatedAt", System.currentTimeMillis());
        new Vault(context).put("stepx-bridge-task-v1", record.toString());
    }
    private static final class BridgeFailure extends Exception {
        final String code;
        BridgeFailure(String code, String message) { super(message); this.code = code; }
    }
}
