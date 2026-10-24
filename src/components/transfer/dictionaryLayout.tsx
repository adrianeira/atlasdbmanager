import { useEffect, useState, type ReactElement } from "react";
import { pg, type DictFont, type DictServerSel, type DictSettings, type DictTemplate } from "../../backend";
import { useT } from "../../i18n";
import { Dropdown } from "../Dropdown";

const TEMPLATES: { id: DictTemplate; key: string }[] = [
  { id: "orange", key: "dd.tpl.orange" },
  { id: "red", key: "dd.tpl.red" },
  { id: "blackWhite", key: "dd.tpl.blackWhite" },
  { id: "blue", key: "dd.tpl.blue" },
  { id: "green", key: "dd.tpl.green" },
];

const FONT_FAMILIES = ["Default", "Arial", "Times New Roman", "Courier New"];
const FONT_SIZES = [8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 20, 22, 24, 28, 32];
const PAPERS = ["A3", "A4", "A5", "Letter", "Legal"];
const ZOOMS = ["whole", "30", "47", "75", "100", "150", "200"];

/** A página desenhada pelo Rust (SVG), no tamanho do zoom: 100% = 96 dpi. */
function PagePreview({ servers, settings, which, zoom }: { servers: DictServerSel[]; settings: DictSettings; which: string; zoom: string }) {
  const [page, setPage] = useState<{ svg: string; width: number; height: number } | null>(null);
  const [box, setBox] = useState<HTMLDivElement | null>(null);
  const [fit, setFit] = useState(0.47);
  useEffect(() => {
    let alive = true;
    const id = window.setTimeout(() => {
      pg.dictPreview(servers, settings, which).then((p) => alive && setPage(p)).catch(() => alive && setPage(null));
    }, 150);
    return () => { alive = false; window.clearTimeout(id); };
  }, [servers, settings, which]);
  useEffect(() => {
    if (!box || !page) return;
    const measure = () => setFit(Math.min((box.clientHeight - 24) / (page.height * (96 / 72)), (box.clientWidth - 24) / (page.width * (96 / 72))));
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(box);
    return () => ro.disconnect();
  }, [box, page]);
  const scale = zoom === "whole" ? fit : Number(zoom) / 100;
  return (
    <div className="dd-preview-scroll" ref={setBox}>
      {page && <div className="dd-page" style={{ width: page.width * (96 / 72) * scale, height: page.height * (96 / 72) * scale }} dangerouslySetInnerHTML={{ __html: page.svg }} />}
    </div>
  );
}

/** Capas de cada modelo, desenhadas pelo mesmo motor do documento. */
function Thumbnail({ servers, settings, template }: { servers: DictServerSel[]; settings: DictSettings; template: DictTemplate }) {
  const [svg, setSvg] = useState("");
  useEffect(() => {
    let alive = true;
    pg.dictPreview(servers, { ...settings, template, includeCover: true }, "cover").then((p) => alive && setSvg(p?.svg ?? "")).catch(() => undefined);
    return () => { alive = false; };
  }, [servers, settings, template]);
  return <div className="dd-thumb" dangerouslySetInnerHTML={{ __html: svg }} />;
}

export function TemplateStep({ servers, settings, onPick }: { servers: DictServerSel[]; settings: DictSettings; onPick: (t: DictTemplate) => void }) {
  const t = useT();
  return (
    <div className="dd-templates">
      {TEMPLATES.map((x) => (
        <button key={x.id} type="button" className={`dd-template${settings.template === x.id ? " is-sel" : ""}`} onClick={() => onPick(x.id)}>
          <Thumbnail servers={servers} settings={settings} template={x.id} />
          <span>{t(x.key as never)}</span>
        </button>
      ))}
    </div>
  );
}

type Tab = "cover" | "toc" | "headerFooter" | "main" | "paper";
const TABS: Tab[] = ["cover", "toc", "headerFooter", "main", "paper"];
const PREVIEW_OF: Record<Tab, string> = { cover: "cover", toc: "toc", headerFooter: "sample", main: "intro", paper: "sample" };

export function LayoutStep({ servers, settings, setSettings, defaults }: { servers: DictServerSel[]; settings: DictSettings; setSettings: (s: DictSettings) => void; defaults: DictSettings }) {
  const t = useT();
  const [tab, setTab] = useState<Tab>("cover");
  const [zoom, setZoom] = useState("47");
  const patch = (p: Partial<DictSettings>) => setSettings({ ...settings, ...p });

  const fontRow = (value: DictFont, onChange: (f: DictFont) => void, disabled = false) => (
    <div className="dd-font-row">
      <Dropdown className="dd-dd" value={value.family} disabled={disabled} items={FONT_FAMILIES.map((f) => ({ value: f, label: t(f === "Default" ? "dd.font.default" : (`dd.font.${f}` as never)) }))} onChange={(v) => onChange({ ...value, family: v })} />
      <Dropdown className="dd-dd dd-size" value={String(value.size)} disabled={disabled} items={FONT_SIZES.map((n) => ({ value: String(n), label: String(n) }))} onChange={(v) => onChange({ ...value, size: Number(v) })} />
    </div>
  );
  const restore = (style: () => Partial<DictSettings>, all: () => Partial<DictSettings>) => (
    <div className="dd-restore">
      <button type="button" className="dd-link" onClick={() => patch(style())}>{t("dd.restoreStyle")}</button>
      <button type="button" className="dd-link" onClick={() => patch(all())}>{t("dd.restoreAll")}</button>
    </div>
  );
  const check = (label: string, value: boolean, onChange: (v: boolean) => void) => (
    <label className="check-row"><input type="checkbox" checked={value} onChange={(e) => onChange(e.target.checked)} /><span>{label}</span></label>
  );
  const margin = (i: number, label: string) => (
    <label className="dd-field"><span>{label}</span>
      <input className="field dd-num" type="number" step="0.1" min="0" value={settings.margins[i]} onChange={(e) => {
        const m = [...settings.margins] as DictSettings["margins"];
        m[i] = Math.max(0, Number(e.target.value) || 0);
        patch({ margins: m });
      }} />
    </label>
  );

  const forms: Record<Tab, ReactElement> = {
    cover: (
      <div className="dd-form">
        {check(t("dd.includeCover"), settings.includeCover, (v) => patch({ includeCover: v }))}
        <label className="dd-field"><span>{t("dd.header")}</span><input className="field" value={settings.coverHeader} onChange={(e) => patch({ coverHeader: e.target.value })} /></label>
        <label className="dd-field"><span>{t("dd.title.field")}</span><input className="field" value={settings.coverTitle} onChange={(e) => patch({ coverTitle: e.target.value })} /></label>
        <label className="dd-field"><span>{t("dd.subtitle")}</span><input className="field" value={settings.coverSubtitle} onChange={(e) => patch({ coverSubtitle: e.target.value })} /></label>
        <label className="dd-field is-top"><span>{t("dd.remarks")}</span><textarea className="field dd-area" value={settings.coverRemarks} spellCheck={false} onChange={(e) => patch({ coverRemarks: e.target.value })} /></label>
        <div className="muted dd-hint">{t("dd.dateFormat")}</div>
        <div className="dd-field is-top"><span>{t("dd.background")}</span>
          <div className="dd-radios">
            <label><input type="radio" name="dd-bg" checked={settings.coverBackground !== "none"} onChange={() => patch({ coverBackground: "default" })} />{t("dd.bgDefault")}</label>
            <label><input type="radio" name="dd-bg" checked={settings.coverBackground === "none"} onChange={() => patch({ coverBackground: "none" })} />{t("dd.bgNone")}</label>
          </div>
        </div>
        {restore(() => ({ template: defaults.template }), () => ({ includeCover: defaults.includeCover, coverHeader: defaults.coverHeader, coverTitle: settings.coverTitle, coverSubtitle: defaults.coverSubtitle, coverRemarks: defaults.coverRemarks, coverBackground: defaults.coverBackground }))}
      </div>
    ),
    toc: (
      <div className="dd-form">
        {check(t("dd.includeToc"), settings.includeToc, (v) => patch({ includeToc: v }))}
        {restore(() => ({}), () => ({ includeToc: defaults.includeToc }))}
      </div>
    ),
    headerFooter: (
      <div className="dd-form">
        {check(t("dd.breadcrumbs"), settings.breadcrumbs, (v) => patch({ breadcrumbs: v }))}
        {fontRow(settings.breadcrumbFont, (f) => patch({ breadcrumbFont: f }), !settings.breadcrumbs)}
        <div className="dd-sublabel">{t("dd.headingFont")}</div>
        {fontRow(settings.headingFont, (f) => patch({ headingFont: f }))}
        <div className="dd-sublabel">{t("dd.subheadingFont")}</div>
        {fontRow(settings.subheadingFont, (f) => patch({ subheadingFont: f }))}
        <div className="dd-left-footer">
          {check(t("dd.leftFooter"), settings.leftFooter, (v) => patch({ leftFooter: v }))}
          <input className="field" value={settings.leftFooterText} disabled={!settings.leftFooter} onChange={(e) => patch({ leftFooterText: e.target.value })} />
        </div>
        {check(t("dd.pageNumber"), settings.pageNumber, (v) => patch({ pageNumber: v }))}
        <div className="dd-sublabel">{t("dd.footerFont")}</div>
        {fontRow(settings.footerFont, (f) => patch({ footerFont: f }))}
        {restore(
          () => ({ breadcrumbFont: defaults.breadcrumbFont, headingFont: defaults.headingFont, subheadingFont: defaults.subheadingFont, footerFont: defaults.footerFont }),
          () => ({ breadcrumbs: defaults.breadcrumbs, breadcrumbFont: defaults.breadcrumbFont, headingFont: defaults.headingFont, subheadingFont: defaults.subheadingFont, leftFooter: defaults.leftFooter, leftFooterText: settings.leftFooterText, pageNumber: defaults.pageNumber, footerFont: defaults.footerFont }),
        )}
      </div>
    ),
    main: (
      <div className="dd-form">
        {check(t("dd.includeIntro"), settings.includeIntro, (v) => patch({ includeIntro: v }))}
        <div className="dd-sublabel">{t("dd.intro")}</div>
        <textarea className="field dd-area dd-intro" value={settings.introText} disabled={!settings.includeIntro} spellCheck={false} onChange={(e) => patch({ introText: e.target.value })} />
        <div className="dd-sublabel">{t("dd.font")}</div>
        <Dropdown className="dd-dd" value={settings.bodyFont} items={FONT_FAMILIES.map((f) => ({ value: f, label: t(f === "Default" ? "dd.font.default" : (`dd.font.${f}` as never)) }))} onChange={(v) => patch({ bodyFont: v })} />
        {restore(() => ({ bodyFont: defaults.bodyFont }), () => ({ includeIntro: defaults.includeIntro, introText: defaults.introText, bodyFont: defaults.bodyFont }))}
      </div>
    ),
    paper: (
      <div className="dd-form">
        <label className="dd-field"><span>{t("dd.paper")}</span>
          <Dropdown className="dd-dd" value={settings.paper} items={PAPERS.map((p) => ({ value: p, label: p }))} onChange={(v) => patch({ paper: v })} />
        </label>
        <div className="dd-sublabel">{t("dd.orientation")}</div>
        <div className="dd-radios">
          <label><input type="radio" name="dd-or" checked={!settings.landscape} onChange={() => patch({ landscape: false })} />{t("dd.portrait")}</label>
          <label><input type="radio" name="dd-or" checked={settings.landscape} onChange={() => patch({ landscape: true })} />{t("dd.landscape")}</label>
        </div>
        <div className="dd-sublabel">{t("dd.margins")}</div>
        {margin(0, t("dd.left"))}{margin(1, t("dd.right"))}{margin(2, t("dd.top"))}{margin(3, t("dd.bottom"))}
        {restore(() => ({}), () => ({ paper: defaults.paper, landscape: defaults.landscape, margins: defaults.margins }))}
      </div>
    ),
  };

  return (
    <div className="dd-layout">
      <div className="dd-tabs" role="tablist">
        {TABS.map((x) => <button key={x} type="button" role="tab" aria-selected={tab === x} className={`dd-tab${tab === x ? " is-active" : ""}`} onClick={() => setTab(x)}>{t(`dd.tab.${x}` as never)}</button>)}
      </div>
      <div className="dd-layout-body">
        <div className="dd-preview-pane">
          <PagePreview servers={servers} settings={settings} which={PREVIEW_OF[tab]} zoom={zoom} />
          <div className="dd-zoom">
            <button type="button" className="icon-button" aria-label="−" onClick={() => setZoom(String(Math.max(20, Math.round((zoom === "whole" ? 47 : Number(zoom)) * 0.8))))}>−</button>
            <button type="button" className="icon-button" aria-label="+" onClick={() => setZoom(String(Math.min(300, Math.round((zoom === "whole" ? 47 : Number(zoom)) * 1.25))))}>+</button>
            <Dropdown className="dd-dd dd-zoom-dd" value={ZOOMS.includes(zoom) ? zoom : "custom"} items={[...ZOOMS.map((z) => ({ value: z, label: z === "whole" ? t("dd.wholePage") : `${z}%` })), ...(ZOOMS.includes(zoom) ? [] : [{ value: "custom", label: `${zoom}%` }])]} onChange={(v) => v !== "custom" && setZoom(v)} />
          </div>
        </div>
        <div className="dd-form-pane">{forms[tab]}</div>
      </div>
    </div>
  );
}
