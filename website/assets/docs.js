/* Documentation sidebar: highlight the section currently in view. */
(function () {
  "use strict";

  const links = Array.from(document.querySelectorAll(".docs-nav a[href^='#']"));
  const sections = links
    .map((link) => document.querySelector(link.getAttribute("href")))
    .filter(Boolean);

  function setActive(id) {
    links.forEach((link) => {
      link.classList.toggle("is-active", link.getAttribute("href") === `#${id}`);
    });
  }

  if (sections.length) {
    setActive(sections[0].id);
    const observer = new IntersectionObserver(
      (entries) => {
        const visible = entries
          .filter((entry) => entry.isIntersecting)
          .sort((a, b) => a.boundingClientRect.top - b.boundingClientRect.top);
        if (visible.length) setActive(visible[0].target.id);
      },
      { rootMargin: "-96px 0px -62% 0px", threshold: 0 }
    );
    sections.forEach((section) => observer.observe(section));

    window.addEventListener("hashchange", () => {
      const id = window.location.hash.slice(1);
      if (id) setActive(id);
    });
  }
})();
