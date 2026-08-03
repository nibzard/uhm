(() => {
  "use strict";

  const mobileQuery = window.matchMedia("(max-width: 48rem)");
  let returnFocusTo = null;

  function setDrawerState(open, { moveFocus = false } = {}) {
    const button = document.getElementById("mobile-menu-button");
    const sidebar = document.querySelector(".sidebar");
    const content = document.querySelector("section.content, .content");
    const cover = document.querySelector("section.cover");
    const label = button?.querySelector(".sr-only");
    const isOpen = Boolean(open && mobileQuery.matches && sidebar);

    document.body.classList.remove("close");
    document.body.classList.toggle("nav-open", isOpen);
    button?.setAttribute("aria-expanded", String(isOpen));
    if (label) label.textContent = isOpen ? "Close navigation" : "Open navigation";

    if (content) content.inert = isOpen;
    if (cover) cover.inert = isOpen;

    if (isOpen && moveFocus) {
      returnFocusTo = button;
      requestAnimationFrame(() => {
        sidebar.querySelector("input, a, button")?.focus();
      });
    } else if (!isOpen && moveFocus && returnFocusTo) {
      returnFocusTo.focus();
      returnFocusTo = null;
    }
  }

  function drawerIsOpen() {
    return mobileQuery.matches && document.body.classList.contains("nav-open");
  }

  function toggleDrawer() {
    setDrawerState(!drawerIsOpen(), { moveFocus: true });
  }

  function focusableDrawerElements() {
    const sidebar = document.querySelector(".sidebar");
    const menuButton = document.getElementById("mobile-menu-button");
    if (!sidebar || !menuButton) return [];

    return [menuButton, ...sidebar.querySelectorAll('a[href], button:not([disabled]), input:not([disabled])')]
      .filter((element) => !element.hidden && element.getClientRects().length > 0);
  }

  function handleDrawerKeys(event) {
    if (!drawerIsOpen()) return;

    if (event.key === "Escape") {
      event.preventDefault();
      setDrawerState(false, { moveFocus: true });
      return;
    }

    if (event.key !== "Tab") return;
    const elements = focusableDrawerElements();
    if (!elements.length) return;
    const first = elements[0];
    const last = elements[elements.length - 1];

    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }

  function setGroupState(button, list, expanded) {
    button.setAttribute("aria-expanded", String(expanded));
    list.hidden = !expanded;
  }

  function enhanceSidebarGroups() {
    const root = document.querySelector(".sidebar-nav > ul");
    if (!root) return;

    Array.from(root.children).forEach((item, index) => {
      const list = Array.from(item.children).find((child) => child.tagName === "UL");
      const strong = item.querySelector(":scope > p > strong, :scope > strong");
      if (!list || (!strong && !item.querySelector(":scope > p > .sidebar-section-toggle"))) return;

      let button = item.querySelector(":scope > p > .sidebar-section-toggle, :scope > .sidebar-section-toggle");
      if (!button) {
        button = document.createElement("button");
        button.type = "button";
        button.className = "sidebar-section-toggle";
        button.textContent = strong.textContent.trim();
        strong.replaceWith(button);
      }

      if (!list.id) list.id = `sidebar-section-${index}`;
      button.setAttribute("aria-controls", list.id);
      const containsActiveRoute = Boolean(list.querySelector("li.active, a.active"));
      const isStartHere = button.textContent.trim().toLowerCase() === "start here";
      setGroupState(button, list, containsActiveRoute || isStartHere);

      if (button.dataset.enhanced) return;
      button.dataset.enhanced = "true";
      button.addEventListener("click", () => {
        setGroupState(button, list, button.getAttribute("aria-expanded") !== "true");
      });
    });
  }

  function enhancePage() {
    const main = document.querySelector("main");
    const sidebar = document.querySelector(".sidebar");
    const sidebarNav = document.querySelector(".sidebar-nav");
    const appName = document.querySelector(".sidebar > h1.app-name");
    const backdrop = document.querySelector(".sidebar-backdrop");

    if (main) main.id = "main-content";
    if (main && backdrop && backdrop.parentElement !== main) main.prepend(backdrop);
    if (sidebar) {
      sidebar.id = "docs-sidebar";
      sidebar.setAttribute("aria-label", "Documentation navigation");
    }
    if (sidebarNav) {
      sidebarNav.setAttribute("role", "navigation");
      sidebarNav.setAttribute("aria-label", "Documentation pages");
    }

    if (appName) {
      const replacement = document.createElement("div");
      replacement.className = appName.className;
      replacement.innerHTML = appName.innerHTML;
      appName.replaceWith(replacement);
    }

    enhanceSidebarGroups();
    document.querySelectorAll(".markdown-section img").forEach((image) => {
      image.loading = "lazy";
      image.decoding = "async";
      image.fetchPriority = "low";
    });
    document.querySelectorAll(".markdown-section pre").forEach((block) => {
      block.tabIndex = 0;
    });
    document.querySelectorAll(".docsify-copy-code-button .error, .docsify-copy-code-button .success")
      .forEach((label) => label.setAttribute("aria-hidden", "true"));
    setDrawerState(false);
    if (!location.hash.includes("?id=")) requestAnimationFrame(() => window.scrollTo(0, 0));
  }

  function bindShell() {
    const menuButton = document.getElementById("mobile-menu-button");
    const backdrop = document.querySelector(".sidebar-backdrop");
    menuButton?.addEventListener("click", toggleDrawer);
    backdrop?.addEventListener("click", () => setDrawerState(false, { moveFocus: true }));
    document.addEventListener("keydown", handleDrawerKeys);
    document.addEventListener("click", (event) => {
      if (drawerIsOpen() && event.target.closest(".sidebar-nav a")) setDrawerState(false);
    });
    mobileQuery.addEventListener("change", () => setDrawerState(false));
  }

  bindShell();

  window.uhmDocsPlugin = function uhmDocsPlugin(hook) {
    hook.ready(enhancePage);
    hook.doneEach(enhancePage);
  };
})();
