const panel = document.querySelector("#delegation-activity");
const list = document.querySelector("#delegation-list");
const count = document.querySelector("#delegation-count");
const collapse = document.querySelector("#delegation-collapse");

const jobs = new Map();
let pollTimer = 0;
let providerCount = 0;

function terminal(status) {
  return status === "succeeded" || status === "failed";
}

function statusLabel(status) {
  return {
    queued: "Queued",
    running: "Working",
    succeeded: "Completed",
    failed: "Failed",
  }[status] || status;
}

function formatTime(value) {
  if (!value) return "";
  return new Date(value).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

function render() {
  const ordered = [...jobs.values()].sort((a, b) => b.createdAt.localeCompare(a.createdAt));
  const active = ordered.filter((job) => !terminal(job.status)).length;
  count.textContent = active
    ? `${active} working`
    : `${providerCount} agent${providerCount === 1 ? "" : "s"} ready`;
  list.replaceChildren();

  if (!ordered.length) {
    const empty = document.createElement("div");
    empty.className = "delegation-empty";
    empty.textContent = providerCount
      ? "Ready. Delegated tasks will appear here."
      : "No ACP agents configured.";
    list.append(empty);
    return;
  }

  for (const job of ordered.slice(0, 8)) {
    const item = document.createElement("article");
    item.className = `delegation-item ${job.status}`;
    const head = document.createElement("div");
    head.className = "delegation-item-head";
    const identity = document.createElement("strong");
    identity.textContent = job.name;
    const state = document.createElement("span");
    state.className = "delegation-state";
    state.textContent = statusLabel(job.status);
    head.append(identity, state);

    const task = document.createElement("p");
    task.className = "delegation-task";
    task.textContent = job.task;
    const detail = document.createElement("p");
    detail.className = "delegation-result";
    detail.textContent = job.status === "failed"
      ? job.error || "The agent failed without an error message."
      : job.status === "succeeded"
        ? job.result || "Completed without a text result."
        : `Started ${formatTime(job.startedAt || job.createdAt)}`;
    item.append(head, task, detail);
    list.append(item);
  }
}

function apply(job, announce = true) {
  const previous = jobs.get(job.id);
  jobs.set(job.id, job);
  render();
  if (announce && previous && !terminal(previous.status) && terminal(job.status)) {
    window.dispatchEvent(new CustomEvent("acp-delegation-complete", { detail: job }));
  }
}

async function refresh() {
  try {
    const response = await fetch("api/delegations", { cache: "no-store" });
    if (!response.ok) return;
    const data = await response.json();
    for (const job of data.delegations || []) apply(job);
  } catch (error) {
    console.warn("Unable to refresh delegated work", error);
  }
  schedule();
}

async function loadProviders() {
  try {
    const response = await fetch("api/acp/providers", { cache: "no-store" });
    if (!response.ok) return;
    const data = await response.json();
    providerCount = (data.providers || []).length;
    render();
  } catch (error) {
    console.warn("Unable to load ACP agent status", error);
  }
}

function schedule() {
  if (pollTimer) window.clearTimeout(pollTimer);
  const hasActive = [...jobs.values()].some((job) => !terminal(job.status));
  pollTimer = hasActive ? window.setTimeout(refresh, 800) : 0;
}

window.addEventListener("acp-delegation-started", (event) => {
  apply(event.detail, false);
  schedule();
});

collapse.addEventListener("click", () => {
  panel.classList.toggle("collapsed");
  collapse.textContent = panel.classList.contains("collapsed") ? "+" : "−";
  collapse.setAttribute("aria-label", panel.classList.contains("collapsed")
    ? "Expand delegated work"
    : "Collapse delegated work");
});

void refresh();
void loadProviders();
