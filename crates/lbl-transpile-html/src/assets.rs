//! Static assets injected into transpiled documents: flex utilities, base CSS,
//! and third-party library references for QR/barcode rendering.

use lbl_text::FontFaceRule;

/// Where third-party JS libraries are loaded from.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum AssetsBase {
    /// Load libraries from a public CDN (jsDelivr).
    #[default]
    Cdn,
    /// Load libraries from a base URL/path serving the vendored assets, e.g.
    /// `/assets` or `file:///opt/lbl/assets`.
    Local(String),
}

impl AssetsBase {
    /// URL for the QR code library (exposes global `QRCode`).
    pub fn qrcode_url(&self) -> String {
        match self {
            AssetsBase::Cdn => "https://cdn.jsdelivr.net/npm/qrcode@1/build/qrcode.min.js".into(),
            AssetsBase::Local(base) => format!("{}/qrcode.min.js", base.trim_end_matches('/')),
        }
    }

    /// URL for the classic 1D barcode library (exposes global `JsBarcode`).
    pub fn jsbarcode_url(&self) -> String {
        match self {
            AssetsBase::Cdn => {
                "https://cdn.jsdelivr.net/npm/jsbarcode@3/dist/JsBarcode.all.min.js".into()
            }
            AssetsBase::Local(base) => {
                format!("{}/JsBarcode.all.min.js", base.trim_end_matches('/'))
            }
        }
    }

    /// URL for bwip-js (exposes global `bwipjs`) — industrial 2D / postal / GS1.
    pub fn bwip_url(&self) -> String {
        match self {
            AssetsBase::Cdn => "https://cdn.jsdelivr.net/npm/bwip-js@4/dist/bwip-js-min.js".into(),
            AssetsBase::Local(base) => {
                format!("{}/bwip-js-min.js", base.trim_end_matches('/'))
            }
        }
    }
}

/// How web fonts referenced by `data-lbl-font` are delivered to the document.
///
/// The engine does not fetch faces. Callers that want named web fonts pass
/// self-describing [`FontFaceRule`]s (URLs or inlined bytes).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum FontDelivery {
    /// No web faces — system stacks only (`sans` / `serif` / `mono`).
    #[default]
    None,
    /// Explicit `@font-face` rules for the slugs used in the document.
    Rules(Vec<FontFaceRule>),
}

/// Base + flex-utility CSS available to every transpiled label.
pub const BASE_CSS: &str = r#"
*,*::before,*::after{box-sizing:border-box}
html,body{margin:0;padding:0}
.lbl-label{display:flex;flex-direction:column}
.lbl-row{display:flex;flex-direction:row;flex-wrap:nowrap;align-items:center}
.lbl-row>.lbl-text{flex:1 1 auto;min-width:0}
.lbl-row>.lbl-barcode,.lbl-row>.lbl-qr,.lbl-row>.lbl-col,.lbl-row>.lbl-slot{flex:0 0 auto;min-width:0}
.lbl-row>.lbl-col,.lbl-row>.lbl-slot{flex:1 1 auto}
.lbl-row .lbl-barcode{width:auto;height:auto}
.lbl-row .lbl-barcode svg{display:block;width:auto;max-width:none;height:auto}
.lbl-row .lbl-qr svg{display:block;width:100%;height:100%;object-fit:contain}
.lbl-col{display:flex;flex-direction:column}
.lbl-slot{flex:1 1 auto;min-width:0;min-height:1em}
.lbl-center{align-items:center;justify-content:center}
.lbl-between{justify-content:space-between}
.lbl-justify-start{justify-content:flex-start}
.lbl-justify-center{justify-content:center}
.lbl-justify-end{justify-content:flex-end}
.lbl-justify-between{justify-content:space-between}
.lbl-justify-around{justify-content:space-around}
.lbl-justify-evenly{justify-content:space-evenly}
.lbl-items-start{align-items:flex-start}
.lbl-items-center{align-items:center}
.lbl-items-end{align-items:flex-end}
.lbl-items-stretch{align-items:stretch}
.lbl-grow{flex:1 1 auto}
.lbl-wrap{flex-wrap:wrap}
.lbl-frame{border:1px solid currentColor;padding:0.5em;box-sizing:border-box}
.lbl-vertical{writing-mode:vertical-rl;text-orientation:upright;display:inline-block;vertical-align:middle;line-height:1}
.lbl-qr,.lbl-barcode{display:inline-flex;align-items:center;justify-content:center}
.lbl-qr canvas,.lbl-qr img,.lbl-qr svg{max-width:100%;max-height:100%;width:auto;height:auto;aspect-ratio:1}
.lbl-barcode svg{display:block;max-width:100%;height:auto}
.lbl-label>img,.lbl-row>img,.lbl-col>img,.lbl-slot>img{display:block;max-width:100%;max-height:100%;width:auto;height:auto;object-fit:contain}
.lbl-label :is(h1,h2,h3,h4,h5,h6,p,ul,ol,blockquote,strong,b,em){margin:0}
.lbl-label h1{font-size:1.35em;font-weight:700}
.lbl-label h2{font-size:1.2em;font-weight:700}
.lbl-label h3{font-size:1.1em;font-weight:700}
.lbl-label h4,.lbl-label h5,.lbl-label h6{font-size:1em;font-weight:700}
.lbl-label p,.lbl-label li{font-size:1em}
"#;

/// Higher-specificity flex utilities on `.lbl-label` so authoring classes win
/// over Fill-mode `label_align` / `label_valign` rules (inject after those).
pub const LABEL_FLEX_OVERRIDE_CSS: &str = r#"
.lbl-label.lbl-justify-start{justify-content:flex-start}
.lbl-label.lbl-justify-center{justify-content:center}
.lbl-label.lbl-justify-end{justify-content:flex-end}
.lbl-label.lbl-justify-between{justify-content:space-between}
.lbl-label.lbl-justify-around{justify-content:space-around}
.lbl-label.lbl-justify-evenly{justify-content:space-evenly}
.lbl-label.lbl-items-start{align-items:flex-start}
.lbl-label.lbl-items-center{align-items:center}
.lbl-label.lbl-items-end{align-items:flex-end}
.lbl-label.lbl-items-stretch{align-items:stretch}
"#;

/// CSS injected for Fill-mode so a lone flex row/col spans the label width.
///
/// Width is stretched (`align-self` + `width:100%`) so nested `lbl-justify-*`
/// runs across the full label. Height stays content-sized (`flex-grow:0`) so
/// `.lbl-label` `lbl-justify-*` can still center/place the block vertically —
/// forcing `height:100%` made the child fill the label and turned outer
/// justify into a no-op (inner `lbl-items-*` then looked like it “owned” the
/// label).
pub const LABEL_FIT_ROW_CSS: &str = r#"
.lbl-label>.lbl-row:only-child,
.lbl-label>.lbl-col:only-child{
  flex:0 1 auto;
  align-self:stretch;
  width:100%;
  min-width:0;
  min-height:0;
  box-sizing:border-box;
}
"#;

/// CSS injected when [`LabelFit::Fill`] is active: stretch the document to the
/// render viewport. Fit-box size, scale, and alignment are set at transpile time.
pub const LABEL_FIT_FILL_CSS: &str = r#"
html,body{height:100%;width:100%;margin:0}
"#;

/// When [`LabelFit::Fill`] is active, grow a lone `.lbl-text` block to use the
/// printable area on fixed die-cut labels. Cross/main-axis alignment is injected
/// at transpile time from [`LabelAlign`] / [`LabelValign`].
///
/// Inline styling (`color`, `font-size`, …) lives in `.lbl-text-inlines` so flex
/// layout does not treat each styled span as its own column item.
pub const LABEL_FIT_TEXT_CSS: &str = r#"
.lbl-label{container-type:size}
.lbl-label>.lbl-text:only-child{
  flex:1 1 auto;
  display:flex;
  flex-direction:column;
  width:100%;
  line-height:1.1;
  font-size:calc(min(calc(100cqh / 1.1), 100cqw) * var(--lbl-font-fit-scale, 1));
  overflow:hidden;
}
.lbl-label>.lbl-text:only-child .lbl-text-inlines{
  display:block;
  width:100%;
  white-space:pre-wrap;
  word-break:break-word;
  line-height:1.1;
}
"#;

/// Line height for row text beside codes in fill mode.
pub const ROW_TEXT_LINE_HEIGHT: f64 = 1.25;

/// When [`LabelFit::Fill`] is active, grow text beside codes in a row
/// (font size is computed at transpile time).
pub const LABEL_FIT_ROW_TEXT_CSS: &str = r#"
.lbl-row>.lbl-text{
  line-height:1.25;
  white-space:nowrap;
  overflow:visible;
  text-align:center;
}
"#;

/// Fill-mode helpers for barcodes/QRs sized via `data-fit-*` attributes.
pub const LABEL_FIT_CODE_CSS: &str = r#"
.lbl-row>.lbl-barcode[data-fit-width],.lbl-row>.lbl-qr[data-fit-width]{
  overflow:visible;
}
.lbl-row>.lbl-barcode[data-fit-width] svg{
  display:block;
  width:100%;
  height:auto;
  max-width:none;
  max-height:none;
}
.lbl-row>.lbl-qr[data-fit-width] svg{
  display:block;
  width:100%;
  height:100%;
  max-width:none;
  max-height:none;
  object-fit:contain;
}
"#;

/// Extra fill rules for preview mode: stretch the gallery wrapper to the
/// viewport so `.lbl-label{height:100%}` has a sized ancestor.
pub const LABEL_FIT_FILL_PREVIEW_CSS: &str = r#"
.lbl-preview{width:100%;height:100%}
"#;

/// Additional CSS for preview mode: a neutral backdrop and a label boundary so
/// the document reads well in a browser/gallery.
pub const PREVIEW_CSS: &str = r#"
body{background:#e9e9ee;display:flex;align-items:center;justify-content:center;min-height:100vh}
.lbl-preview{background:#fff;box-shadow:0 1px 6px rgba(0,0,0,.25)}
"#;

/// JS that renders QR placeholders (`.lbl-qr[data-qr]`) as SVG.
///
/// Honors `window.__LBL_STYLE.qr.width` (pixels) when present, so the rendered
/// QR matches the configured physical size. SVG keeps modules sharp in both
/// raster screenshots and vector PDF export.
pub const QR_INIT_JS: &str = r#"
(function(){
  function render(){
    var st=(window.__LBL_STYLE&&window.__LBL_STYLE.qr)||{};
    document.querySelectorAll('.lbl-qr').forEach(function(el){
      if(el.dataset.rendered) return;
      var value = el.getAttribute('data-qr') || '';
      if(window.QRCode && QRCode.toString){
        var opts={type:'svg',margin:0};
        if(st.width){opts.width=st.width;}
        if(st.errorCorrectionLevel){opts.errorCorrectionLevel=st.errorCorrectionLevel;}
        if(typeof st.margin==='number'){opts.margin=st.margin;}
        if(st.color){opts.color=st.color;}
        var ec=el.getAttribute('data-ec');
        if(ec){opts.errorCorrectionLevel=ec;}
        var margin=el.getAttribute('data-margin');
        if(margin!==null && margin!==''){opts.margin=parseInt(margin,10);}
        var width=el.getAttribute('data-width');
        if(width!==null && width!==''){opts.width=parseInt(width,10);}
        var fitW=el.getAttribute('data-fit-width');
        if((width===null || width==='') && fitW!==null && fitW!==''){opts.width=parseInt(fitW,10);}
        var dark=el.getAttribute('data-dark');
        var light=el.getAttribute('data-light');
        if(dark||light){
          opts.color=Object.assign({},opts.color||{});
          if(dark){opts.color.dark=dark;}
          if(light){opts.color.light=light;}
        }
        QRCode.toString(value, opts, function(err, svg){
          if(!err && svg){ el.innerHTML = svg; }
          el.dataset.rendered = '1';
        });
      } else {
        el.dataset.rendered = '1';
      }
    });
  }
  if(document.readyState!=='loading'){render();}else{document.addEventListener('DOMContentLoaded',render);}
})();
"#;

/// JS that renders barcode placeholders (`.lbl-barcode[data-value]`).
///
/// Dispatches on `data-renderer`: `jsbarcode` (default) or `bwip`. Honors
/// `window.__LBL_STYLE.barcode` (`width`/`height`/`fontSize`, all in pixels)
/// when present. Bar and caption colors inherit from surrounding CSS `color`.
pub const BARCODE_INIT_JS: &str = r#"
(function(){
  function cssColorToHex(c){
    if(!c) return null;
    var m=String(c).match(/^rgba?\((\d+),\s*(\d+),\s*(\d+)/i);
    if(!m) return null;
    function h(n){var s=Number(n).toString(16);return s.length<2?'0'+s:s;}
    return h(m[1])+h(m[2])+h(m[3]);
  }
  function renderJsBarcode(el,st){
    if(!window.JsBarcode) return false;
    var svg=document.createElementNS('http://www.w3.org/2000/svg','svg');
    el.appendChild(svg);
    var opts={format:el.getAttribute('data-symbology')||'CODE128',margin:0,textMargin:0,displayValue:true};
    if(st.width){opts.width=st.width;}
    if(st.height){opts.height=st.height;}
    if(st.fontSize){opts.fontSize=st.fontSize;}
    var fitW=el.getAttribute('data-fit-width');
    var fitH=el.getAttribute('data-fit-height');
    var baseH=st.height||100;
    var baseFont=st.fontSize||20;
    if(fitH!==null&&fitH!==''){
      opts.height=parseInt(fitH,10);
      var fitFont=el.getAttribute('data-fit-font-size');
      if(fitFont!==null&&fitFont!==''){opts.fontSize=parseInt(fitFont,10);}
      else{opts.fontSize=Math.max(8,Math.round(baseFont*(opts.height/baseH)));}
    }
    var color=window.getComputedStyle(el).color;
    if(color){opts.lineColor=color;opts.textColor=color;}
    var value=el.getAttribute('data-value')||'';
    try{
      if(fitW!==null&&fitW!==''){
        var targetW=parseInt(fitW,10);
        var moduleW=st.width||2;
        opts.width=moduleW;
        JsBarcode(svg,value,opts);
        var bbox=svg.getBBox&&svg.getBBox();
        var curW=(bbox&&bbox.width)||svg.width.baseVal.value||0;
        if(targetW>0){
          if(curW>0){opts.width=Math.max(0.5,moduleW*(targetW/curW));}
          else{var estModules=Math.max(20,value.length*11+36);opts.width=Math.max(0.5,targetW/estModules);}
          svg.innerHTML='';
          JsBarcode(svg,value,opts);
        }
      }else{JsBarcode(svg,value,opts);}
    }catch(e){}
    if(!svg.childNodes.length){svg.remove();return false;}
    return true;
  }
  function renderBwip(el,st){
    if(!window.bwipjs||!window.bwipjs.toSVG) return false;
    var value=el.getAttribute('data-value')||'';
    var bcid=el.getAttribute('data-bcid');
    if(!bcid) return false;
    var is2d=el.getAttribute('data-barcode-2d')==='1';
    var opts={bcid:bcid,text:value,scale:2,includetext:!is2d};
    var fitW=el.getAttribute('data-fit-width');
    var fitH=el.getAttribute('data-fit-height');
    if(fitH!==null&&fitH!==''&&!is2d){
      var hPx=parseInt(fitH,10);
      if(hPx>0){opts.height=Math.max(1,hPx/((window.devicePixelRatio||1)*3.78));}
    }else if(st.height&&!is2d){
      opts.height=Math.max(1,(st.height)/((window.devicePixelRatio||1)*3.78));
    }
    var color=window.getComputedStyle(el).color;
    var hex=cssColorToHex(color);
    if(hex){opts.barcolor=hex;opts.textcolor=hex;}
    try{
      var svgStr=bwipjs.toSVG(opts);
      if(fitW!==null&&fitW!==''){
        var targetW=parseInt(fitW,10);
        var m=/viewBox="0 0 ([\d.]+) ([\d.]+)"/.exec(svgStr);
        if(m&&targetW>0){
          var natW=parseFloat(m[1]);
          if(natW>0){
            opts.scale=Math.max(1,(opts.scale||2)*(targetW/natW));
            svgStr=bwipjs.toSVG(opts);
          }
        }
      }
      el.innerHTML=svgStr;
      var svg=el.querySelector('svg');
      if(svg){
        svg.style.display='block';
        svg.style.maxWidth='100%';
        svg.style.height='auto';
      }
      return !!svg;
    }catch(e){return false;}
  }
  function render(){
    var st=(window.__LBL_STYLE&&window.__LBL_STYLE.barcode)||{};
    document.querySelectorAll('.lbl-barcode').forEach(function(el){
      if(el.dataset.rendered) return;
      var renderer=(el.getAttribute('data-renderer')||'jsbarcode').toLowerCase();
      var ok=renderer==='bwip'?renderBwip(el,st):renderJsBarcode(el,st);
      if(ok){el.dataset.rendered='1';}
    });
  }
  if(document.readyState!=='loading'){render();}else{document.addEventListener('DOMContentLoaded',render);}
})();
"#;

#[cfg(test)]
mod tests {
    use super::BASE_CSS;

    #[test]
    fn base_css_includes_justify_and_items_modifiers() {
        for class in [
            "lbl-justify-start",
            "lbl-justify-center",
            "lbl-justify-end",
            "lbl-justify-between",
            "lbl-justify-around",
            "lbl-justify-evenly",
            "lbl-items-start",
            "lbl-items-center",
            "lbl-items-end",
            "lbl-items-stretch",
        ] {
            assert!(BASE_CSS.contains(class), "BASE_CSS missing .{class}");
        }
    }

    #[test]
    fn base_css_includes_slot_and_frame() {
        assert!(BASE_CSS.contains(".lbl-slot{"));
        assert!(BASE_CSS.contains(".lbl-frame{"));
        assert!(BASE_CSS.contains(".lbl-row>.lbl-slot"));
    }

    #[test]
    fn base_css_includes_vertical_text() {
        assert!(BASE_CSS.contains(".lbl-vertical{"));
        assert!(BASE_CSS.contains("writing-mode:vertical-rl"));
        assert!(BASE_CSS.contains("text-orientation:upright"));
        assert!(BASE_CSS.contains("vertical-align:middle"));
        assert!(BASE_CSS.contains("line-height:1"));
    }

    #[test]
    fn base_css_keeps_legacy_center_and_between() {
        assert!(BASE_CSS.contains(".lbl-center{"));
        assert!(BASE_CSS.contains(".lbl-between{"));
    }

    #[test]
    fn fill_row_css_stretches_lone_flex_child() {
        use super::LABEL_FIT_ROW_CSS;
        assert!(LABEL_FIT_ROW_CSS.contains("align-self:stretch"));
        assert!(LABEL_FIT_ROW_CSS.contains("width:100%"));
        assert!(LABEL_FIT_ROW_CSS.contains("flex:0 1 auto"));
        assert!(!LABEL_FIT_ROW_CSS.contains("height:100%"));
        assert!(LABEL_FIT_ROW_CSS.contains(".lbl-label>.lbl-row:only-child"));
        assert!(LABEL_FIT_ROW_CSS.contains(".lbl-label>.lbl-col:only-child"));
    }

    #[test]
    fn label_flex_override_targets_lbl_label_modifiers() {
        use super::LABEL_FLEX_OVERRIDE_CSS;
        assert!(LABEL_FLEX_OVERRIDE_CSS.contains(".lbl-label.lbl-justify-center"));
        assert!(LABEL_FLEX_OVERRIDE_CSS.contains(".lbl-label.lbl-items-stretch"));
    }
}
