// Progressive enhancement for .tour pages (framework-in-brief):
// a section progress rail and a subtle reveal-on-scroll.
// No-JS or reduced-motion readers get the full static page.
// Deliberately avoids requestAnimationFrame and IntersectionObserver so it
// also behaves in throttled/automation environments and print pipelines.
(function () {
    "use strict";

    var tour = document.querySelector(".tour");
    if (!tour) {
        return;
    }

    var reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;

    // ---- Section deck -----------------------------------------------------
    // Every section whose heading is followed by an .unpacks chip collapses
    // to a header strip (heading + chip); the body opens on click. Bodies
    // collapse via height:0, not display:none, for the same mermaid-width
    // reason as .fold. Anchor navigation (rail dots, "section N" links,
    // search) auto-expands the target section. No-JS readers get the full
    // static page.
    var secs = [];

    Array.prototype.slice
        .call(tour.querySelectorAll("h2[id]"))
        .forEach(function (h2) {
            var chip = h2.nextElementSibling;
            if (!chip || !chip.classList.contains("unpacks")) {
                return;
            }
            var sec = document.createElement("section");
            sec.className = "sec sec--closed";
            h2.parentNode.insertBefore(sec, h2);

            var head = document.createElement("div");
            head.className = "sec-head";
            head.setAttribute("role", "button");
            head.setAttribute("tabindex", "0");
            var chev = document.createElement("span");
            chev.className = "sec-chev";
            head.appendChild(h2);
            head.appendChild(chip);
            head.appendChild(chev);

            var numMatch = h2.textContent.match(/^\s*(\d+)/);
            if (numMatch) {
                var num = document.createElement("span");
                num.className = "sec-num";
                num.textContent = numMatch[1];
                sec.appendChild(num);
            }

            var body = document.createElement("div");
            body.className = "sec-body";
            sec.appendChild(head);
            var node = sec.nextSibling;
            while (node && !(node.nodeType === 1 && node.tagName === "H2")) {
                var next = node.nextSibling;
                if (node.nodeType === 1 && node.classList.contains("slide") && !body.children.length) {
                    sec.appendChild(node);
                } else {
                    body.appendChild(node);
                }
                node = next;
            }
            sec.appendChild(body);

            function render() {
                var open = !sec.classList.contains("sec--closed");
                chev.textContent = open ? "▾" : "▸";
                head.setAttribute("aria-expanded", String(open));
            }
            function toggle() {
                sec.classList.toggle("sec--closed");
                render();
                renderDeckControl();
                onScroll();
            }
            function clickToggle(event) {
                if (event.target.closest && event.target.closest("a")) {
                    return;
                }
                toggle();
            }
            head.addEventListener("click", clickToggle);
            head.addEventListener("keydown", function (event) {
                if (event.key === "Enter" || event.key === " ") {
                    event.preventDefault();
                    toggle();
                }
            });
            var slide = sec.querySelector(".slide");
            if (slide) {
                slide.addEventListener("click", clickToggle);
            }
            render();
            secs.push({ sec: sec, render: render });
        });

    var deckControl = null;
    function renderDeckControl() {
        if (!deckControl) {
            return;
        }
        var anyClosed = secs.some(function (s) {
            return s.sec.classList.contains("sec--closed");
        });
        deckControl.textContent = anyClosed ? "▸  expand all sections" : "▾  collapse all sections";
    }

    if (secs.length) {
        var controls = document.createElement("div");
        controls.className = "sec-controls";
        deckControl = document.createElement("button");
        deckControl.type = "button";
        deckControl.className = "sec-controls__toggle";
        deckControl.addEventListener("click", function () {
            var anyClosed = secs.some(function (s) {
                return s.sec.classList.contains("sec--closed");
            });
            secs.forEach(function (s) {
                s.sec.classList.toggle("sec--closed", !anyClosed);
                s.render();
            });
            renderDeckControl();
            onScroll();
        });
        controls.appendChild(deckControl);
        secs[0].sec.parentNode.insertBefore(controls, secs[0].sec);
        renderDeckControl();
    }

    function openSectionFor(hash) {
        if (!hash || hash.length < 2) {
            return;
        }
        var target;
        try {
            target = document.getElementById(decodeURIComponent(hash.slice(1)));
        } catch (e) {
            return;
        }
        if (!target) {
            return;
        }
        secs.forEach(function (s) {
            if (s.sec.contains(target) && s.sec.classList.contains("sec--closed")) {
                s.sec.classList.remove("sec--closed");
                s.render();
                renderDeckControl();
                window.setTimeout(function () {
                    target.scrollIntoView();
                    onScroll();
                }, 0);
            }
        });
    }
    window.addEventListener("hashchange", function () {
        openSectionFor(window.location.hash);
    });
    openSectionFor(window.location.hash);

    // ---- Section progress rail -------------------------------------------
    var headings = Array.prototype.slice.call(tour.querySelectorAll("h2[id]"));
    var dots = [];
    if (headings.length >= 4) {
        var rail = document.createElement("nav");
        rail.className = "tour-rail";
        rail.setAttribute("aria-label", "Sections");

        dots = headings.map(function (heading, index) {
            var dot = document.createElement("a");
            dot.href = "#" + heading.id;
            var label = heading.textContent.trim();
            dot.textContent = String(index + 1);
            dot.setAttribute("data-label", label);
            dot.setAttribute("aria-label", label);
            rail.appendChild(dot);
            return dot;
        });

        document.body.appendChild(rail);
    }

    var active = -1;
    function updateRail() {
        if (!dots.length) {
            return;
        }
        var cutoff = window.innerHeight * 0.34;
        var current = -1;
        for (var i = 0; i < headings.length; i += 1) {
            if (headings[i].getBoundingClientRect().top <= cutoff) {
                current = i;
            }
        }
        if (current === active) {
            return;
        }
        if (active >= 0) {
            dots[active].classList.remove("on");
        }
        active = current;
        if (active >= 0) {
            dots[active].classList.add("on");
        }
    }

    // ---- Reveal-on-scroll -------------------------------------------------
    var pending = [];
    if (!reducedMotion) {
        pending = Array.prototype.slice.call(tour.children).filter(function (el) {
            var tag = el.tagName;
            return tag === "PRE" || tag === "TABLE" || tag === "UL" || tag === "OL" ||
                el.classList.contains("duo") || el.classList.contains("facts") ||
                el.classList.contains("seq") || el.classList.contains("mermaid");
        });
        pending.forEach(function (el) {
            el.classList.add("reveal");
        });
    }

    function updateReveals() {
        if (!pending.length) {
            return;
        }
        var limit = window.innerHeight * 0.96;
        pending = pending.filter(function (el) {
            var rect = el.getBoundingClientRect();
            if (rect.top <= limit && rect.bottom >= 0) {
                el.classList.add("on");
                return false;
            }
            return true;
        });
    }

    // ---- Collapsible diagrams ---------------------------------------------
    // Each .fold wraps one diagram; a toggle button replaces it until clicked.
    // Collapsed via height:0 (not display:none) so mermaid still renders at
    // real width while hidden. Without JS the diagrams stay fully visible.
    Array.prototype.slice.call(tour.querySelectorAll(".fold")).forEach(function (fold) {
        var label = fold.getAttribute("data-label") || "diagram";
        var body = document.createElement("div");
        body.className = "fold__body";
        while (fold.firstChild) {
            body.appendChild(fold.firstChild);
        }
        var toggle = document.createElement("button");
        toggle.type = "button";
        toggle.className = "fold__toggle";
        fold.appendChild(toggle);
        fold.appendChild(body);
        fold.classList.add("fold--closed");

        function render() {
            var open = !fold.classList.contains("fold--closed");
            toggle.textContent = (open ? "▾  " : "▸  ") + label;
            toggle.setAttribute("aria-expanded", String(open));
        }
        toggle.addEventListener("click", function () {
            fold.classList.toggle("fold--closed");
            render();
            onScroll();
        });
        render();
    });

    // ---- Architecture map: click opens a zoomed, pannable overlay ---------
    // Clicking a §N badge navigates to its section instead; a drag pans the
    // zoomed map without closing it.
    var archMap = tour.querySelector(".arch-map svg");
    if (archMap) {
        archMap.style.cursor = "zoom-in";
        archMap.addEventListener("click", function (event) {
            if (event.target.closest && event.target.closest("a")) {
                return;
            }
            openArchMapOverlay(archMap, event);
        });
    }

    function openArchMapOverlay(svg, event) {
        var overlay = document.createElement("div");
        overlay.className = "arch-map-overlay";
        overlay.setAttribute("role", "dialog");
        overlay.setAttribute("aria-modal", "true");
        overlay.setAttribute("aria-label", "Architecture map");
        var content = document.createElement("div");
        content.className = "arch-map-overlay__content";
        var controls = document.createElement("div");
        controls.className = "arch-map-overlay__controls";
        controls.innerHTML = "<span>Drag to pan · Esc to close</span>";
        var closeButton = document.createElement("button");
        closeButton.type = "button";
        closeButton.textContent = "Close";
        closeButton.setAttribute("aria-label", "Close architecture map");
        controls.appendChild(closeButton);

        var zoomWidth = Math.min(1700, Math.round(window.innerWidth * 1.9));
        overlay.style.setProperty("--fw-zoom-width", zoomWidth + "px");

        var clone = svg.cloneNode(true);
        clone.style.display = "block";
        content.appendChild(clone);
        overlay.appendChild(controls);
        overlay.appendChild(content);
        document.body.appendChild(overlay);
        var previousFocus = document.activeElement;
        closeButton.focus();

        // Scroll the overlay so the clicked point sits centered.
        var rect = svg.getBoundingClientRect();
        var fx = (event.clientX - rect.left) / rect.width;
        var fy = (event.clientY - rect.top) / rect.height;
        var zoomHeight = zoomWidth * (rect.height / rect.width);
        content.scrollLeft = Math.max(0, fx * zoomWidth - content.clientWidth / 2);
        content.scrollTop = Math.max(0, fy * zoomHeight - content.clientHeight / 2);

        var dragging = false;
        var moved = false;
        var startX = 0;
        var startY = 0;
        var startLeft = 0;
        var startTop = 0;
        overlay.addEventListener("pointerdown", function (e) {
            dragging = true;
            moved = false;
            startX = e.clientX;
            startY = e.clientY;
            startLeft = content.scrollLeft;
            startTop = content.scrollTop;
        });
        overlay.addEventListener("pointermove", function (e) {
            if (!dragging) {
                return;
            }
            var dx = e.clientX - startX;
            var dy = e.clientY - startY;
            if (Math.abs(dx) + Math.abs(dy) > 6) {
                moved = true;
            }
            content.scrollLeft = startLeft - dx;
            content.scrollTop = startTop - dy;
        });
        overlay.addEventListener("pointerup", function () {
            dragging = false;
        });

        function onKey(e) {
            if (e.key === "Escape") {
                close();
            }
        }
        function close() {
            overlay.remove();
            document.removeEventListener("keydown", onKey);
            if (previousFocus && previousFocus.focus) {
                previousFocus.focus();
            }
        }
        controls.addEventListener("pointerdown", function (e) {
            e.stopPropagation();
        });
        controls.addEventListener("click", function (e) {
            e.stopPropagation();
        });
        closeButton.addEventListener("click", function (e) {
            e.stopPropagation();
            close();
        });
        overlay.addEventListener("click", function () {
            if (moved) {
                moved = false;
                return;
            }
            close();
        });
        document.addEventListener("keydown", onKey);
    }

    // ---- One throttled driver for both -----------------------------------
    var ticking = false;
    function update() {
        updateRail();
        updateReveals();
    }

    function onScroll() {
        if (ticking) {
            return;
        }
        ticking = true;
        window.setTimeout(function () {
            ticking = false;
            update();
        }, 60);
    }

    window.addEventListener("scroll", onScroll, { passive: true });
    window.addEventListener("resize", onScroll, { passive: true });

    // Initial pass: mark what is already in view (mermaid renders async and
    // shifts layout, so run again shortly after load).
    update();
    window.setTimeout(update, 400);
    window.setTimeout(update, 1500);
})();
