"use strict";

// Shared keyboard behaviour for both sets of tabs (WAI-ARIA automatic activation).
document.querySelectorAll('[role="tablist"]').forEach((list) => {
  const tabs = [...list.querySelectorAll('[role="tab"]')];
  const activate = (tab, focus = false) => {
    tabs.forEach((item) => {
      const selected = item === tab;
      item.setAttribute("aria-selected", String(selected));
      item.tabIndex = selected ? 0 : -1;
      document.getElementById(item.getAttribute("aria-controls")).hidden =
        !selected;
    });
    if (focus) tab.focus();
  };
  tabs.forEach((tab, index) => {
    tab.addEventListener("click", () => activate(tab));
    tab.addEventListener("keydown", (event) => {
      let next;
      if (event.key === "ArrowRight") next = (index + 1) % tabs.length;
      if (event.key === "ArrowLeft")
        next = (index - 1 + tabs.length) % tabs.length;
      if (event.key === "Home") next = 0;
      if (event.key === "End") next = tabs.length - 1;
      if (next !== undefined) {
        event.preventDefault();
        activate(tabs[next], true);
      }
    });
  });
});

const dialog = document.querySelector(".image-dialog");
const fullImage = dialog.querySelector(".dialog-image");
document.querySelectorAll(".screenshot-trigger").forEach((button) => {
  button.addEventListener("click", () => {
    fullImage.src = button.dataset.image;
    fullImage.alt = button.querySelector("img").alt;
    dialog.querySelector(".dialog-caption").textContent =
      button.dataset.caption;
    dialog.showModal();
    dialog.querySelector(".dialog-image-wrap").scrollTo(0, 0);
  });
});
dialog
  .querySelector(".dialog-close")
  .addEventListener("click", () => dialog.close());
dialog.addEventListener("click", (event) => {
  if (event.target !== dialog) return;
  const box = dialog.getBoundingClientRect();
  if (
    event.clientX < box.left ||
    event.clientX > box.right ||
    event.clientY < box.top ||
    event.clientY > box.bottom
  )
    dialog.close();
});
dialog.addEventListener("close", () => {
  fullImage.removeAttribute("src");
});

let statusTimer;
const status = document.querySelector(".copy-status");
document.querySelectorAll(".copy-button").forEach((button) => {
  button.addEventListener("click", async () => {
    const code = document.getElementById(button.dataset.copy);
    button.disabled = true;
    try {
      if (!navigator.clipboard) throw new Error("Clipboard unavailable");
      await navigator.clipboard.writeText(code.textContent);
      status.textContent = "Commands copied. Ready when you are.";
    } catch {
      const range = document.createRange();
      range.selectNodeContents(code);
      const selection = window.getSelection();
      selection.removeAllRanges();
      selection.addRange(range);
      status.textContent = "Commands selected. Press Ctrl+C or ⌘C to copy.";
    } finally {
      button.disabled = false;
      clearTimeout(statusTimer);
      statusTimer = setTimeout(() => {
        status.textContent = "";
      }, 5000);
    }
  });
});
