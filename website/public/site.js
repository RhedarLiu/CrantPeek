"use strict";

// Native details keeps the download choices usable without JavaScript.
const menu = document.querySelector(".download-menu");
const toggle = menu.querySelector("summary");
const links = [...menu.querySelectorAll(".menu-panel a")];

document.addEventListener("click", (event) => {
  if (!menu.contains(event.target)) menu.open = false;
});
menu.addEventListener("keydown", (event) => {
  if (event.key === "Escape") {
    menu.open = false;
    toggle.focus();
    event.preventDefault();
  } else if (event.key === "ArrowDown" || event.key === "ArrowUp") {
    event.preventDefault();
    menu.open = true;
    const current = links.indexOf(document.activeElement);
    const next = current < 0
      ? (event.key === "ArrowDown" ? 0 : links.length - 1)
      : (current + (event.key === "ArrowDown" ? 1 : -1) + links.length) % links.length;
    links[next].focus();
  }
});
menu.addEventListener("focusout", () => {
  setTimeout(() => {
    if (!menu.contains(document.activeElement)) menu.open = false;
  }, 0);
});
