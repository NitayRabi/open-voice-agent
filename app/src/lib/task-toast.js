export class TaskToast {
  constructor({ compact = false } = {}) {
    this.hideTimer = null;
    this.el = document.createElement("div");
    this.el.className = `task-toast${compact ? " compact" : ""}`;
    this.el.hidden = true;
    this.el.setAttribute("role", "status");
    this.el.setAttribute("aria-live", "polite");
    this.el.innerHTML = '<span class="task-toast-icon" aria-hidden="true"></span><span class="task-toast-copy"><strong></strong><small></small></span>';
    document.body.append(this.el);
  }

  update(task) {
    clearTimeout(this.hideTimer);
    const title = this.el.querySelector("strong");
    const detail = this.el.querySelector("small");
    this.el.className = `task-toast${this.el.classList.contains("compact") ? " compact" : ""} status-${task.status}`;
    this.el.hidden = false;

    if (task.status === "running") {
      title.textContent = "Task sent to agent";
      detail.textContent = task.request || "Working in the background…";
      return;
    }
    if (task.status === "completed") {
      title.textContent = "Agent task complete";
      detail.textContent = task.result || "Result ready";
    } else {
      title.textContent = "Agent task failed";
      detail.textContent = task.error || "The task could not be completed";
    }
    this.hideTimer = setTimeout(() => (this.el.hidden = true), 6000);
  }
}
