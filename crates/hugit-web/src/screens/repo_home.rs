//! Repo home — GitHub-faithful code home.
//! Spec: `../../../../../githugr/design/repo-home.html`

use crate::provider::RepoHomeVm;
use maud::{Markup, PreEscaped, html};

/// Screen-specific CSS — everything from the mockup's `<style>` block that is
/// NOT in `static/kit.css` (the kit owns only topbar/tabbar/footer/⌘K).
/// Rules are scoped under `.rhome` to avoid body-level conflicts.
pub const SCREEN_CSS: &str = r#"
/* ---- repo header (GitHub-class) ---- */
.rhome .repohdr{padding:14px 22px 0;border-bottom:1px solid var(--line);background:var(--rail)}
.rhome .rh1{display:flex;align-items:center;gap:10px;flex-wrap:wrap}
.rhome .rh1 .path{font-size:16px;color:var(--muted)}
.rhome .rh1 .path b{color:var(--text);font-weight:600}
.rhome .rh1 .vis{font-size:11px;color:var(--faint);border:1px solid var(--line-2);border-radius:20px;padding:1px 9px}
.rhome .rh1 .sp{flex:1}
.rhome .rh1 .act{display:inline-flex;align-items:center;gap:6px;border:1px solid var(--line-2);border-radius:6px;padding:4px 11px;color:var(--text-2);font-size:12.5px;cursor:pointer}
.rhome .rh1 .act:hover{border-color:var(--line-3)}
.rhome .rh1 .act .n{color:var(--muted)}
.rhome .rh1 .clone{background:var(--accent);border-color:var(--accent);color:#15151a;font-weight:600}
.rhome .rdesc{color:var(--muted);font-size:13px;margin:9px 0 14px;max-width:70ch}
/* clone dropdown */
.rhome .clone-wrap{position:relative}
.rhome .clone-dd{position:absolute;top:calc(100% + 6px);right:0;width:320px;background:var(--panel-2);border:1px solid var(--line-3);border-radius:9px;box-shadow:0 16px 50px -10px rgba(0,0,0,.75);z-index:30;padding:14px;display:none}
.rhome .clone-dd.show{display:block}
.rhome .clone-dd .lbl{font-size:10.5px;letter-spacing:.07em;text-transform:uppercase;color:var(--faint);font-weight:600;margin-bottom:8px}
.rhome .clone-dd .cmd{display:flex;align-items:center;gap:8px;background:rgba(255,255,255,.04);border:1px solid var(--line-2);border-radius:6px;padding:8px 11px;font-family:var(--mono);font-size:12px;color:var(--text-2)}
.rhome .clone-dd .cmd .sp{flex:1}
.rhome .clone-dd .cp{cursor:pointer;color:var(--muted);border:1px solid var(--line-2);border-radius:5px;padding:2px 7px;font-size:11px;font-family:var(--font)}
.rhome .clone-dd .cp:hover{color:var(--text);border-color:var(--line-3)}
/* ---- body grid ---- */
.rhome .rhbody{display:grid;grid-template-columns:1fr 280px;overflow:hidden;min-height:0}
.rhome .rhmain{overflow-y:auto;padding:18px 22px}
/* branch bar */
.rhome .branchbar{display:flex;align-items:center;gap:10px;margin-bottom:14px}
.rhome .branchsel{display:inline-flex;align-items:center;gap:7px;border:1px solid var(--line-2);border-radius:6px;padding:5px 11px;color:var(--text-2);font-size:12.5px;cursor:pointer}
.rhome .branchsel .ic{color:var(--faint)}
.rhome .branchbar .sp{flex:1}
.rhome .branchbar .gi{color:var(--faint);font-size:12.5px}
.rhome .gofile{border:1px solid var(--line-2);border-radius:6px;padding:5px 11px;color:var(--faint);font-size:12.5px;cursor:pointer}
.rhome .gofile:hover{border-color:var(--line-3);color:var(--muted)}
/* branch dropdown */
.rhome .branch-wrap{position:relative}
.rhome .branch-dd{position:absolute;top:calc(100% + 6px);left:0;width:200px;background:var(--panel-2);border:1px solid var(--line-3);border-radius:9px;box-shadow:0 16px 50px -10px rgba(0,0,0,.75);z-index:30;padding:8px 0;display:none}
.rhome .branch-dd.show{display:block}
.rhome .branch-dd .bi{padding:7px 14px;font-size:13px;color:var(--muted);cursor:pointer}
.rhome .branch-dd .bi:hover{background:var(--hover);color:var(--text)}
.rhome .branch-dd .bi.cur{color:var(--text);font-weight:500}
/* file table */
.rhome .files{border:1px solid var(--line);border-radius:var(--r);overflow:hidden}
.rhome .frow{display:grid;grid-template-columns:24px 1fr auto auto;align-items:center;gap:12px;padding:8px 14px;border-bottom:1px solid var(--line);font-size:13px;cursor:pointer}
.rhome .frow:last-child{border-bottom:0}
.rhome .frow:hover{background:var(--hover)}
.rhome .frow .ic{color:var(--faint);text-align:center}
.rhome .frow .nm{color:var(--text-2)}
.rhome .frow .msg{color:var(--faint);font-size:12.5px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;max-width:320px}
.rhome .frow .tm{color:var(--dim);font-size:12px;white-space:nowrap}
.rhome .frow .msg .ix{color:var(--accent);font-family:var(--mono);font-size:11px;opacity:.8}
/* README panel */
.rhome .readme{margin-top:20px;border:1px solid var(--line);border-radius:var(--r);overflow:hidden}
.rhome .readme .rhd{display:flex;align-items:center;gap:8px;padding:10px 16px;border-bottom:1px solid var(--line);background:var(--rail);color:var(--muted);font-size:12.5px;font-weight:500}
.rhome .readme .rbody{padding:22px 26px}
.rhome .readme .rbody h1{font-size:24px;font-weight:700;margin:0 0 6px;letter-spacing:-.02em}
.rhome .readme .rbody p{color:var(--text-2);margin:10px 0;font-size:14px}
.rhome .readme .rbody h2{font-size:16px;font-weight:600;margin:22px 0 6px;padding-bottom:6px;border-bottom:1px solid var(--line)}
.rhome .readme .rbody code{font-family:var(--mono);font-size:12.5px;background:rgba(255,255,255,.05);border:1px solid var(--line);border-radius:5px;padding:1px 6px;color:var(--text-2)}
/* About sidebar */
.rhome .aside{border-left:1px solid var(--line);background:var(--rail);overflow-y:auto;padding:18px 16px}
.rhome .ahd{font-size:13px;font-weight:600;margin-bottom:8px}
.rhome .ap{color:var(--muted);font-size:12.5px;margin-bottom:14px}
.rhome .topics{display:flex;flex-wrap:wrap;gap:6px;margin-bottom:16px}
.rhome .topic{font-size:11.5px;color:var(--accent);background:var(--accent-soft);border-radius:20px;padding:2px 10px}
.rhome .ameta{display:flex;flex-direction:column;gap:9px;font-size:12.5px;color:var(--muted);border-top:1px solid var(--line);padding-top:14px}
.rhome .ameta .r{display:flex;align-items:center;gap:9px}
.rhome .ameta .r .ic{color:var(--faint);width:15px}
.rhome .ameta .r b{color:var(--text-2);font-weight:500}
.rhome .asec{border-top:1px solid var(--line);padding:14px 0 0;margin-top:14px}
.rhome .asec .t{font-size:13px;font-weight:600;margin-bottom:9px}
.rhome .asec .lk{color:var(--muted);font-size:12.5px;display:flex;align-items:center;gap:7px}
.rhome .contrib{display:flex;gap:5px;margin-top:4px}
.rhome .contrib .a{width:26px;height:26px;border-radius:50%;background:#2a2a31;border:1px solid var(--line-2);display:inline-flex;align-items:center;justify-content:center;font-size:10px;color:var(--faint)}
/* synergy panel */
.rhome .synergy{margin-top:16px;border:1px solid var(--line);border-radius:var(--r);background:var(--panel);padding:13px}
.rhome .synergy .h{font-size:10.5px;letter-spacing:.06em;text-transform:uppercase;color:var(--faint);font-weight:600;margin-bottom:9px}
.rhome .synergy .r{display:flex;align-items:center;gap:9px;font-size:12.5px;color:var(--muted);padding:4px 0}
.rhome .synergy .r .v{margin-left:auto;color:var(--text-2);font-weight:500}
/* toast */
.rh-toast{position:fixed;bottom:54px;left:50%;transform:translateX(-50%) translateY(8px);background:var(--panel-2);border:1px solid var(--line-3);border-radius:8px;padding:9px 18px;font-size:13px;color:var(--text-2);box-shadow:0 8px 30px rgba(0,0,0,.6);opacity:0;transition:opacity .18s,transform .18s;pointer-events:none;z-index:100;white-space:nowrap}
.rh-toast.show{opacity:1;transform:translateX(-50%) translateY(0)}
/* go-to-file overlay */
.rh-gfov{position:fixed;inset:0;background:rgba(6,6,8,.6);backdrop-filter:blur(2px);display:none;align-items:flex-start;justify-content:center;padding-top:14vh;z-index:50}
.rh-gfov.show{display:flex}
.rh-gfpal{width:560px;max-width:92vw;background:var(--panel-2);border:1px solid var(--line-3);border-radius:11px;box-shadow:0 24px 70px -20px rgba(0,0,0,.8);overflow:hidden}
.rh-gfpal .gpin{display:flex;align-items:center;gap:11px;padding:14px 16px;border-bottom:1px solid var(--line)}
.rh-gfpal .gpin input{flex:1;background:none;border:0;outline:0;color:var(--text);font-size:15px;font-family:var(--font)}
.rh-gfpal .gpin input::placeholder{color:var(--faint)}
.rh-gfpal .gpin .ic{color:var(--faint)}
.rh-gflist{max-height:340px;overflow-y:auto}
.rh-gfitem{display:flex;align-items:center;gap:11px;padding:9px 16px;color:var(--muted);cursor:pointer;font-size:13px}
.rh-gfitem .fn{font-family:var(--mono);font-size:12.5px;color:var(--text-2)}
.rh-gfitem .fp{font-size:11.5px;color:var(--faint);margin-left:4px}
.rh-gfitem.sel{background:var(--accent-soft);color:var(--text)}
.rh-gfitem.sel .fn{color:var(--text)}
.rh-gfitem:hover{background:var(--hover)}
.rh-gfoot{padding:8px 16px;border-top:1px solid var(--line);font-size:11px;color:var(--faint);display:flex;gap:16px}
"#;

pub fn render(vm: &RepoHomeVm) -> Markup {
    html! {
        div .rhome {
            // Repo header row (GitHub-class)
            div .repohdr {
                div .rh1 {
                    span .path {
                        "humangr / " b { (vm.repo) }
                    }
                    span .vis { "Privado" }
                    span .sp {}
                    span .act title="read-only" { span .n { "★" } " Star" }
                    span .act title="read-only" { span .n { "⑂" } " Fork" }
                    div .clone-wrap {
                        span .act .clone
                            onclick="rhToggleClone(event)"
                            title="read-only" {
                            "⟱ Code ▾"
                        }
                        div .clone-dd #rh-clone-dd {
                            div .lbl { "Clonar com hugit" }
                            div .cmd {
                                span { "hugit clone humangr/" (vm.repo) }
                                span .sp {}
                                span .cp onclick="rhCopyClone(event,(this))" { "copiar" }
                            }
                        }
                    }
                }
                div .rdesc {
                    "Versionado por "
                    b style="color:var(--text-2)" { "hugit" }
                    ", hospedado no "
                    b style="color:var(--text-2)" { "githugr" }
                    ", espelhado no GitHub."
                }
            }

            // Body: code home
            div .rhbody {
                div .rhmain {
                    // Branch bar
                    div .branchbar {
                        div .branch-wrap {
                            span .branchsel onclick="rhToggleBranch(event)" {
                                span .ic { "⑂" }
                                " " (vm.branch) " "
                                span style="color:var(--faint)" { "▾" }
                            }
                            div .branch-dd #rh-branch-dd {
                                div .bi .cur { "⑂ " (vm.branch) }
                            }
                        }
                        span .gi {
                            b style="color:var(--text-2)" { (vm.branch_count) }
                            " Branches"
                        }
                        span .sp {}
                        span .gofile onclick="rhOpenGoFile()" {
                            "Go to file " kbd { "t" }
                        }
                    }

                    // File table
                    div .files {
                        @for f in &vm.files {
                            div .frow {
                                // Icon: directory or file
                                @if f.is_dir {
                                    span .ic { "▸" }
                                } @else {
                                    span .ic { "▤" }
                                }
                                span .nm {
                                    @if f.is_dir {
                                        (f.name) "/"
                                    } @else {
                                        (f.name)
                                    }
                                }
                                span .msg {
                                    (f.message)
                                    // Intent link when present
                                    @if let Some(ref id) = f.intent_id {
                                        " "
                                        a .ix
                                            href={ "/r/" (vm.repo) "/intent/" (id) } {
                                            "· " (id)
                                        }
                                    }
                                }
                                span .tm { (f.age) }
                            }
                        }
                    }

                    // README preview panel
                    div .readme {
                        div .rhd { "▤ README.md" }
                        div .rbody {
                            (PreEscaped(&vm.readme_html))
                        }
                    }
                }

                // About sidebar
                div .aside {
                    div .ahd { "About" }
                    div .ap { (vm.about.description) }

                    @if !vm.about.topics.is_empty() {
                        div .topics {
                            @for topic in &vm.about.topics {
                                span .topic { (topic) }
                            }
                        }
                    }

                    div .ameta {
                        @if let Some(ref rel) = vm.about.release {
                            div .r {
                                span .ic { "◷" }
                                " release " b { (rel) }
                            }
                        }
                        @if !vm.about.contributors.is_empty() {
                            div .r {
                                span .ic { "▤" }
                                " contribuidores"
                            }
                        }
                    }

                    @if !vm.about.contributors.is_empty() {
                        div .asec {
                            div .t {
                                "Contributors"
                            }
                            div .contrib {
                                @for c in &vm.about.contributors {
                                    span .a title=(c) {
                                        (c.chars().next().unwrap_or('?').to_uppercase().to_string())
                                    }
                                }
                            }
                        }
                    }

                    // Synergy panel
                    div .synergy {
                        div .h { "⟂ camada hugit (ao vivo)" }
                        @for (label, value) in &vm.synergy.lines {
                            div .r {
                                (label)
                                span .v { (value) }
                            }
                        }
                    }
                }
            }

            // Go-to-file overlay (t key)
            div .rh-gfov #rh-gfov onclick="if(event.target===this)rhCloseGoFile()" {
                div .rh-gfpal {
                    div .gpin {
                        span .ic { "▤" }
                        input #rh-gfq
                            placeholder="Ir a arquivo…"
                            oninput="rhFilterFiles()"
                            onkeydown="rhGfKey(event)";
                    }
                    div .rh-gflist #rh-gflist {}
                    div .rh-gfoot {
                        span { kbd { "↑↓" } " navegar" }
                        span { kbd { "↵" } " abrir" }
                        span { kbd { "Esc" } " fechar" }
                    }
                }
            }

            // Toast
            div .rh-toast #rh-toast {}

            // Inline JS for this screen
            script {
                (PreEscaped(SCREEN_JS))
            }
        }
    }
}

const SCREEN_JS: &str = r#"
(function(){
  /* ---- toast ---- */
  var _rht;
  function rhToast(msg){
    var t=document.getElementById('rh-toast');
    t.textContent=msg;t.classList.add('show');
    clearTimeout(_rht);_rht=setTimeout(()=>t.classList.remove('show'),2600);
  }
  window.rhToast=rhToast;

  /* ---- clone dropdown ---- */
  window.rhToggleClone=function(e){
    e.stopPropagation();
    document.getElementById('rh-clone-dd').classList.toggle('show');
  };
  window.rhCopyClone=function(e){
    e.stopPropagation();
    var repo=e.target.closest('.clone-dd').querySelector('.cmd span').textContent;
    navigator.clipboard&&navigator.clipboard.writeText(repo).catch(()=>{});
    document.getElementById('rh-clone-dd').classList.remove('show');
    rhToast('Copiado: '+repo);
  };

  /* ---- branch dropdown ---- */
  window.rhToggleBranch=function(e){
    e.stopPropagation();
    document.getElementById('rh-branch-dd').classList.toggle('show');
  };
  document.addEventListener('click',function(e){
    var bd=document.getElementById('rh-branch-dd');
    if(bd&&!bd.closest('.branch-wrap').contains(e.target))bd.classList.remove('show');
    var cd=document.getElementById('rh-clone-dd');
    if(cd&&!cd.closest('.clone-wrap').contains(e.target))cd.classList.remove('show');
  });

  /* ---- go-to-file (t key) ---- */
  var GF_FILES=[];
  (function(){
    var rows=document.querySelectorAll('.rhome .frow .nm');
    rows.forEach(function(n){GF_FILES.push({nm:n.textContent.trim(),path:''});});
  })();
  var _gfSel=0,_gfFiltered=GF_FILES.slice();

  window.rhOpenGoFile=function(){
    _gfFiltered=GF_FILES.slice();_gfSel=0;
    document.getElementById('rh-gfov').classList.add('show');
    document.getElementById('rh-gfq').value='';
    rhRenderGF();
    setTimeout(()=>document.getElementById('rh-gfq').focus(),30);
  };
  window.rhCloseGoFile=function(){
    document.getElementById('rh-gfov').classList.remove('show');
  };
  window.rhFilterFiles=function(){
    var q=document.getElementById('rh-gfq').value.toLowerCase();
    _gfFiltered=GF_FILES.filter(f=>(f.path+f.nm).toLowerCase().includes(q));
    _gfSel=0;rhRenderGF();
  };
  function rhRenderGF(){
    var list=document.getElementById('rh-gflist');
    if(!_gfFiltered.length){
      list.innerHTML='<div style="padding:18px 16px;color:var(--faint);font-size:13px">Nenhum arquivo encontrado</div>';
      return;
    }
    list.innerHTML=_gfFiltered.map((f,i)=>
      '<div class="rh-gfitem'+(i===_gfSel?' sel':'')+'" onclick="rhToast(\'navega na árvore — demo mostra a raiz\')">'
      +'<span class="fn">'+f.nm+'</span>'
      +(f.path?'<span class="fp">'+f.path+'</span>':'')
      +'</div>'
    ).join('');
  }
  window.rhGfKey=function(e){
    if(e.key==='ArrowDown'){e.preventDefault();_gfSel=Math.min(_gfSel+1,_gfFiltered.length-1);rhRenderGF();}
    else if(e.key==='ArrowUp'){e.preventDefault();_gfSel=Math.max(_gfSel-1,0);rhRenderGF();}
    else if(e.key==='Enter'){e.preventDefault();rhCloseGoFile();}
    else if(e.key==='Escape'){rhCloseGoFile();}
  };

  /* ---- global t key → go to file ---- */
  document.addEventListener('keydown',function(e){
    var tag=document.activeElement.tagName;
    var inInput=(tag==='INPUT'||tag==='TEXTAREA'||document.activeElement.isContentEditable);
    var kovOpen=document.getElementById('kov')&&document.getElementById('kov').classList.contains('open');
    if(e.key.toLowerCase()==='t'&&!inInput&&!kovOpen){e.preventDefault();rhOpenGoFile();}
    if(e.key==='Escape')rhCloseGoFile();
  });
})();
"#;
