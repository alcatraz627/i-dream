"""Exercise the round-4 page against the fifteen callouts and screenshot every surface in both themes."""
import sys
from pathlib import Path
from playwright.sync_api import sync_playwright

out = Path(sys.argv[1]); out.mkdir(parents=True, exist_ok=True)
url = sys.argv[2]
report = []
def check(name, ok, detail=""):
    report.append(f"{'PASS' if ok else 'FAIL'} {name} {detail}")

with sync_playwright() as p:
    b = p.chromium.launch(channel="chrome", headless=True)
    pg = b.new_page(viewport={"width": 1320, "height": 900}, device_scale_factor=2)
    errors = []; pg.on("pageerror", lambda e: errors.append(str(e)))
    pg.goto(url); pg.wait_for_timeout(1200)
    check("F1 assertion banner hidden", not pg.locator("#assert").evaluate("e=>e.classList.contains('on')"), pg.locator("#assert").text_content())
    # F1: totals equal rows
    led_total = pg.locator("#c-led").text_content().replace(",", "")
    pg.locator(".side .nav[data-pane='led']").click(); pg.wait_for_timeout(300)
    rows_sum = pg.evaluate("() => [...document.querySelectorAll('#pane-led tr.r')].filter(r=>!r.textContent.includes('retired')).reduce((a,r)=>a+parseInt(r.children[4].textContent.replace(/[^0-9]/g,'')||0),0)")
    check("F1 ledger total equals live row sum", int(led_total) == rows_sum, f"{led_total} vs {rows_sum}")
    found_chip = pg.locator("#c-read").text_content()
    pg.locator(".side .nav[data-pane='read']").click(); pg.wait_for_timeout(300)
    cards = pg.locator("#cards .cl:not(.noise)").count()
    check("F1 findings count equals cards", int(found_chip) == cards, f"{found_chip} vs {cards}")
    # F10: run click re-renders
    pg.locator("tr.r[data-run='W37']").click(); pg.wait_for_timeout(300)
    check("F10 run click loads that run", "W37" in pg.locator("#pane-read .sub").text_content() and pg.locator("#cards .cl").count() == 2)
    pg.locator("tr.r[data-run='W40']").click(); pg.wait_for_timeout(200)
    # F4: every dropdown row dives, scoped
    pg.locator("#pop .prow").first.click(); pg.wait_for_timeout(300)
    check("F4 You row opens Landing with the item selected", pg.locator("#pane-land").evaluate("e=>e.classList.contains('on')") and pg.locator("#pane-land .chain.cur").count() == 1)
    pg.locator("#pop .prow").nth(2).click(); pg.wait_for_timeout(300)
    check("F4 System row opens Ledger scoped to the domain", pg.locator("#pane-led").evaluate("e=>e.classList.contains('on')") and pg.locator("#pane-led tr.r.open").count() == 1)
    pg.locator("#pop .box").nth(1).click(); pg.wait_for_timeout(200)
    check("F4 ribbon stage opens Reader", pg.locator("#pane-read").evaluate("e=>e.classList.contains('on')"))
    # F5: retired excluded
    pop_text = pg.locator("#pop").text_content()
    check("F5 retired row is grey and not in totals", "retired source" in pop_text and "4,052" not in pg.locator("#pop .ribbon").text_content())
    # F3 on the default (last) cycle, before any click
    pg.locator(".side .nav[data-pane='flow']").click(); pg.wait_for_timeout(400)
    check("F3 idle cycle reads idle", "idle" in pg.locator("#log").text_content(), pg.locator("#log .k").text_content())
    # F9: drag re-aggregates
    before = pg.locator("#pane-flow .stage").nth(1).locator(".big").text_content()
    bb = pg.locator("#hypF svg").bounding_box()
    pg.mouse.move(bb["x"] + bb["width"]*0.75, bb["y"] + 60); pg.mouse.down(); pg.mouse.move(bb["x"] + bb["width"]*0.95, bb["y"] + 60, steps=8); pg.mouse.up(); pg.wait_for_timeout(400)
    after = pg.locator("#pane-flow .stage").nth(1).locator(".big").text_content()
    check("F9 drag changes the Patterns stage count", before != after, f"{before!r} -> {after!r}")
    pg.keyboard.press("Escape"); pg.wait_for_timeout(300)
    # F9: cycle click loads its log
    pg.locator("#hypF rect.seg").nth(5).click(); pg.wait_for_timeout(200)
    check("F9 cycle click loads that cycle's log", "1857" in pg.locator("#log .k").text_content(), pg.locator("#log .k").text_content())
    # F8: stale OR dead with empty state
    pg.locator(".side .nav[data-pane='led']").click(); pg.wait_for_timeout(300)
    if pg.locator("#pane-led [data-cleardom]").count(): pg.locator("#pane-led [data-cleardom]").click(); pg.wait_for_timeout(200)
    pg.locator("#pane-led .chip[data-f='stale']").click(); pg.wait_for_timeout(200); pg.locator("#pane-led .chip[data-f='dead']").click(); pg.wait_for_timeout(200)
    n = pg.locator("#pane-led tr.r").count()
    check("F8 stale OR dead shows the stale rows (no dead lane)", n == 2, f"{n} rows")
    pg.locator("#pane-led .chip[data-f='idle']").click(); pg.wait_for_timeout(200)
    check("F8 chips show on state", pg.locator("#pane-led .chip.on").count() == 3)
    # F7: patterns chips compose, no placeholders, copy
    pg.locator(".side .nav[data-pane='pat']").click(); pg.wait_for_timeout(400)
    names = pg.evaluate("() => [...document.querySelectorAll('#plist .pitem:not(.dim) .nm')].map(n=>n.firstChild.textContent)")
    check("F7 no placeholder names", not any(nm.startswith("pattern-") for nm in names), f"{len(names)} names")
    pg.locator("#pane-pat .chip[data-trend='worsening']").click(); pg.wait_for_timeout(200); pg.locator("#pane-pat .chip[data-cat='voice']").click(); pg.wait_for_timeout(200)
    vis = pg.locator("#plist .pitem:not(.dim)").count()
    check("F7 worsening AND voice composes", vis == 3, f"{vis} rows, chips on={pg.locator('#pane-pat .chip.on').count()}")
    pg.locator("#pane-pat .chip[data-cat='voice']").click(); pg.locator("#pane-pat .chip[data-trend='worsening']").click(); pg.wait_for_timeout(200)
    pg.locator("#plist .pitem").first.click(); pg.wait_for_timeout(300)
    check("F7 card has full slug and scale", "top" in pg.locator("#plist .pcard .meta").first.text_content())
    # F11: keys
    pg.keyboard.press("j"); pg.keyboard.press("j"); pg.wait_for_timeout(100)
    check("F11 j moves in Patterns", pg.locator("#plist .pitem.cur").count() == 1)
    pg.locator(".side .nav[data-pane='read']").click(); pg.wait_for_timeout(300); pg.keyboard.press("j"); pg.keyboard.press("Enter"); pg.wait_for_timeout(200)
    check("F11 j+enter opens a Reader card", pg.locator("#cards .cl.open").count() == 1, f"cur={pg.locator('#pane-read .cur').count()} cards={pg.locator('#cards .cl').count()} open={pg.locator('#cards .cl.open').count()}")
    pg.locator(".side .nav[data-pane='land']").click(); pg.wait_for_timeout(300); pg.keyboard.press("j"); pg.keyboard.press("Enter"); pg.wait_for_timeout(200)
    check("F11 j+enter opens a Landing chain", pg.locator("#why").is_visible())
    # F15: chain node dives
    pg.locator("#chains .chain").first.locator("[data-node='finding']").click(); pg.wait_for_timeout(300)
    check("F15 finding node opens Reader scoped", pg.locator("#pane-read").evaluate("e=>e.classList.contains('on')") and pg.locator("#pane-read .chip[data-clearslug]").count() == 1)
    # F13: no design annotations
    txt = pg.locator("#win").text_content()
    check("F13 no 'situate' annotation on the product", "situate" not in txt)
    # F14: Quiet has no worsening
    quiet = pg.evaluate("() => { const s=[...document.querySelectorAll('#pop .sec')]; const q=s.find(x=>x.textContent.trim()==='Quiet'); let t=''; let n=q.nextElementSibling; while(n && !n.classList.contains('sec') && !n.classList.contains('foot')){ t+=n.textContent; n=n.nextElementSibling; } return t; }")
    check("F14 Quiet holds no worsening item", "worsening" not in quiet)
    # F2: effect without landing reads unattributed
    pg.locator(".side .nav[data-pane='flow']").click(); pg.wait_for_timeout(300)
    check("F2 unattributed effects say so", "unattributed" in pg.locator("#pane-flow").text_content())
    # F3: idle reading
    print("\n".join(report)); print("page errors:", errors or "none"); report.clear()
    # shots
    pg.locator(".side .nav[data-pane='led']").click(); pg.wait_for_timeout(200)
    pg.evaluate("() => { document.querySelectorAll('#pane-led .chip.on').forEach(c=>c.click()); }"); pg.wait_for_timeout(300)
    for theme in ("dark", "light"):
        pg.evaluate(f"document.documentElement.dataset.theme='{theme}'"); pg.wait_for_timeout(250)
        pg.locator("#bar").screenshot(path=str(out / f"r4-bar-{theme}.png"))
        pg.locator("#pop").screenshot(path=str(out / f"r4-pop-{theme}.png"))
        for pane in ("flow", "led", "pat", "read", "land"):
            pg.locator(f".side .nav[data-pane='{pane}']").click(); pg.wait_for_timeout(400)
            if pane == "land": pg.locator("#chains .chain").first.click(); pg.wait_for_timeout(200)
            if pane == "read": pg.locator("#cards .cl").first.click(); pg.wait_for_timeout(200)
            if pane == "pat": pg.locator("#plist .pitem").first.click(); pg.wait_for_timeout(300)
            if pane == "led": pg.locator("#pane-led tr.r").nth(1).click(); pg.wait_for_timeout(200)
            pg.locator("#win").screenshot(path=str(out / f"r4-{pane}-{theme}.png"))
    print("shots done; page errors:", errors or "none")
    b.close()
