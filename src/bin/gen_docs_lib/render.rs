//! Render Catalog into a single self-contained HTML string using maud.

use crate::lib::model::{Catalog, UnitKind};
use maud::{html, Markup, DOCTYPE, PreEscaped};

const CSS: &str = include_str!("render_style.css");
const JS:  &str = include_str!("render_script.js");

pub fn page(c: &Catalog) -> String {
    let tower_count = c.units.iter().filter(|u| u.kind == UnitKind::Tower).count();
    let hero_count = c.units.iter().filter(|u| u.kind == UnitKind::Hero).count();
    let creep_count = c.units.iter().filter(|u| u.kind == UnitKind::Creep).count();

    let page: Markup = html! {
        (DOCTYPE)
        html lang="zh-Hant" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { "omoba · Unit & Script API Catalog" }
                style { (PreEscaped(CSS)) }
            }
            body {
                header.topbar {
                    div.title { "omoba catalog" }
                    div.meta {
                        span { "story: " (c.meta.story) }
                        span { "git: " (c.meta.git_sha) }
                        span { "built: " (c.meta.timestamp) }
                    }
                    div.controls {
                        input #q type="search" placeholder="🔍 filter units / methods";
                        label { input #only-used type="checkbox"; " show only used" }
                        label { input #dark type="checkbox"; " dark" }
                    }
                }

                @if !c.warnings.is_empty() {
                    section.warnings {
                        h2 { "Warnings (" (c.warnings.len()) ")" }
                        ul {
                            @for w in &c.warnings {
                                li { strong { (w.source) } ": " (w.message) }
                            }
                        }
                    }
                }

                div.layout {
                    nav.sidebar {
                        h3 { "Units" }
                        ul {
                            li { a href="#towers" { "Towers (" (tower_count) ")" } }
                            li { a href="#heroes" { "Heroes (" (hero_count) ")" } }
                            li { a href="#creeps" { "Creeps (" (creep_count) ")" } }
                        }
                        h3 { "API" }
                        ul {
                            li { a href="#abilities" { "Abilities (" (c.abilities.len()) ")" } }
                            li { a href="#unit-hooks" { "UnitScript Hooks (" (c.api.unit_hooks.len()) ")" } }
                            li { a href="#ability-hooks" { "AbilityScript (" (c.api.ability_hooks.len()) ")" } }
                            li { a href="#world" { "GameWorld (" (c.api.world_methods.len()) ")" } }
                            li { a href="#stat-keys" { "Stat Keys (" (c.api.stat_keys.len()) ")" } }
                        }
                        h3 { "Report" }
                        ul {
                            li { a href="#coverage" { "Coverage Matrix" } }
                        }
                    }
                    main.content {
                        (section_units(c))
                        (section_abilities(c))
                        (section_api(c))
                        (section_stat_keys(c))
                        (section_coverage(c))
                    }
                }

                footer.footer {
                    "sources: " (c.meta.sources.join(" · "))
                }
                script { (PreEscaped(JS)) }
            }
        }
    };
    page.into_string()
}

fn section_units(_c: &Catalog) -> Markup { html! { section #units { h2 { "Units" } p { "(coming in next task)" } } } }
fn section_abilities(_c: &Catalog) -> Markup { html! { section #abilities { h2 { "Abilities" } p { "(coming)" } } } }
fn section_api(_c: &Catalog) -> Markup { html! { section #api { h2 { "Script API" } p { "(coming)" } } } }
fn section_stat_keys(_c: &Catalog) -> Markup { html! { section #stat-keys { h2 { "Stat Keys" } p { "(coming)" } } } }
fn section_coverage(_c: &Catalog) -> Markup { html! { section #coverage { h2 { "Coverage Matrix" } p { "(coming)" } } } }
