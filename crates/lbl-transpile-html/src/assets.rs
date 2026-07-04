//! Static assets injected into transpiled documents: flex utilities, base CSS,
//! and third-party library references for QR/barcode rendering.

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

    /// URL for the barcode library (exposes global `JsBarcode`).
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
}

/// Base + flex-utility CSS available to every transpiled label.
pub const BASE_CSS: &str = r#"
*,*::before,*::after{box-sizing:border-box}
html,body{margin:0;padding:0}
.lbl-label{display:flex;flex-direction:column}
.lbl-row{display:flex;flex-direction:row;flex-wrap:nowrap;align-items:center}
.lbl-row>.lbl-text,.lbl-row>.lbl-barcode,.lbl-row>.lbl-qr,.lbl-row>.lbl-col{flex:0 0 auto;min-width:0}
.lbl-row>.lbl-col{flex:1 1 auto}
.lbl-row .lbl-barcode{width:auto;height:auto}
.lbl-row .lbl-barcode svg{display:block;width:auto;max-width:none;height:auto}
.lbl-col{display:flex;flex-direction:column}
.lbl-center{align-items:center;justify-content:center}
.lbl-between{justify-content:space-between}
.lbl-grow{flex:1 1 auto}
.lbl-wrap{flex-wrap:wrap}
.lbl-qr,.lbl-barcode{display:inline-flex;align-items:center;justify-content:center}
.lbl-qr canvas,.lbl-qr img,.lbl-qr svg{max-width:100%;height:auto}
.lbl-barcode svg{display:block;max-width:100%;height:auto}
.lbl-label :is(h1,h2,h3,h4,h5,h6,p,ul,ol,blockquote,strong,b,em){margin:0}
.lbl-label h1{font-size:1.35em;font-weight:700}
.lbl-label h2{font-size:1.2em;font-weight:700}
.lbl-label h3{font-size:1.1em;font-weight:700}
.lbl-label h4,.lbl-label h5,.lbl-label h6{font-size:1em;font-weight:700}
.lbl-label p,.lbl-label li{font-size:1em}
"#;

/// CSS injected when [`LabelFit::Fill`] is active: stretch the document to the
/// render viewport. Fit-box size, scale, and alignment are set at transpile time.
pub const LABEL_FIT_FILL_CSS: &str = r#"
html,body{height:100%;width:100%;margin:0}
"#;

/// When [`LabelFit::Fill`] is active, grow a lone `.lbl-text` block to use the
/// printable area on fixed die-cut labels. Cross/main-axis alignment is injected
/// at transpile time from [`LabelAlign`] / [`LabelValign`].
pub const LABEL_FIT_TEXT_CSS: &str = r#"
.lbl-label{container-type:size}
.lbl-label>.lbl-text:only-child{
  flex:1 1 auto;
  display:flex;
  flex-direction:column;
  width:100%;
  line-height:1.1;
  font-size:min(calc(100cqh / 1.1),100cqw);
  white-space:pre-wrap;
  overflow:hidden;
  word-break:break-word;
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
        var dark=el.getAttribute('data-dark');
        var light=el.getAttribute('data-light');
        if(dark||light){
          opts.color=Object.assign({},opts.color||{});
          if(dark){opts.color.dark=dark;}
          if(light){opts.color.light=light;}
        }
        QRCode.toString(value, opts, function(err, svg){
          if(!err && svg){ el.innerHTML = svg; }
        });
      }
      el.dataset.rendered = '1';
    });
  }
  if(document.readyState!=='loading'){render();}else{document.addEventListener('DOMContentLoaded',render);}
})();
"#;

/// JS that renders barcode placeholders (`.lbl-barcode[data-value]`).
///
/// Honors `window.__LBL_STYLE.barcode` (`width`/`height`/`fontSize`, all in
/// pixels) when present, so the rendered barcode matches the configured size.
pub const BARCODE_INIT_JS: &str = r#"
(function(){
  function render(){
    var st=(window.__LBL_STYLE&&window.__LBL_STYLE.barcode)||{};
    document.querySelectorAll('.lbl-barcode').forEach(function(el){
      if(el.dataset.rendered) return;
      var svg = document.createElementNS('http://www.w3.org/2000/svg','svg');
      el.appendChild(svg);
      if(window.JsBarcode){
        var opts={format: el.getAttribute('data-symbology')||'CODE128', margin:0};
        if(st.width){opts.width=st.width;}
        if(st.height){opts.height=st.height;}
        if(st.fontSize){opts.fontSize=st.fontSize;}
        try{ JsBarcode(svg, el.getAttribute('data-value')||'', opts); }catch(e){}
      }
      el.dataset.rendered = '1';
    });
  }
  if(document.readyState!=='loading'){render();}else{document.addEventListener('DOMContentLoaded',render);}
})();
"#;
