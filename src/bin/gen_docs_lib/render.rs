//! Render Catalog into a single self-contained HTML string using maud.

use crate::lib::model::{Catalog, UnitKind};
use crate::lib::model::UnitEntry;
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

fn section_units(c: &Catalog) -> Markup {
    let towers: Vec<&UnitEntry> = c.units.iter().filter(|u| u.kind == UnitKind::Tower).collect();
    let heroes: Vec<&UnitEntry> = c.units.iter().filter(|u| u.kind == UnitKind::Hero).collect();
    let creeps: Vec<&UnitEntry> = c.units.iter().filter(|u| u.kind == UnitKind::Creep).collect();
    html! {
        section #towers {
            h2 { "Towers (" (towers.len()) ")" }
            @for u in towers { (tower_card(u)) }
        }
        section #heroes {
            h2 { "Heroes (" (heroes.len()) ")" }
            @for u in heroes { (hero_card(u)) }
        }
        section #creeps {
            h2 { "Creeps (" (creeps.len()) ")" }
            @for u in creeps { (creep_card(u)) }
        }
    }
}

fn tower_card(u: &UnitEntry) -> Markup {
    let t = u.tower.as_ref().cloned().unwrap_or_default();
    let search = format!("{} {} tower", u.id, t.label);
    html! {
        div.card data-search=(search) {
            h3 { (t.label) " " span.sub { "(" (u.id) ")" } }
            dl.kv {
                dt { "atk" } dd { (t.atk) }
                dt { "range" } dd { (t.range) }
                dt { "asd" } dd { (t.asd_interval) "s" }
                dt { "bullet speed" } dd { (t.bullet_speed) }
                dt { "splash / hit r" } dd { (t.splash_radius) " / " (t.hit_radius) }
                dt { "slow" } dd { "×" (t.slow_factor) " · " (t.slow_duration) "s" }
                dt { "cost" } dd { (t.cost) }
                dt { "hp / footprint" } dd { (t.hp) " / " (t.footprint) }
            }
            (impl_block(u))
        }
    }
}

fn hero_card(u: &UnitEntry) -> Markup {
    let h = match &u.hero { Some(h) => h.clone(), None => return html!{} };
    let search = format!("{} {} hero {}", u.id, h.name, h.title);
    html! {
        div.card data-search=(search) {
            h3 { (h.name) " " span.sub { "— " (h.title) " · " (u.id) } }
            p.sub { (h.background) }
            dl.kv {
                dt { "attrs (S/A/I)" } dd { (h.strength) " / " (h.agility) " / " (h.intelligence)
                    " (" (h.primary_attribute) ")" }
                dt { "hp / mana" } dd { (h.base_hp) " / " (h.base_mana) }
                dt { "dmg / range" } dd { (h.base_damage) " / " (h.attack_range) }
                dt { "armor" } dd { (h.base_armor) }
                dt { "move / turn" } dd { (h.move_speed) " / " (h.turn_speed) }
            }
            @if !u.abilities.is_empty() {
                div.tags {
                    @for a in &u.abilities { span.tag.ability { (a) } }
                }
            }
            @if !h.level_growth.is_null() {
                details { summary { "level growth" }
                    pre.mono { (serde_json::to_string_pretty(&h.level_growth).unwrap_or_default()) }
                }
            }
            (impl_block(u))
        }
    }
}

fn creep_card(u: &UnitEntry) -> Markup {
    let c = match &u.creep { Some(c) => c.clone(), None => return html!{} };
    let search = format!("{} {} creep {}", u.id, c.name, c.enemy_type);
    html! {
        div.card data-search=(search) {
            h3 { (c.name) " " span.sub { "(" (u.id) " · " (c.enemy_type) ")" } }
            dl.kv {
                dt { "hp / armor / mr" } dd { (c.hp) " / " (c.armor) " / " (c.magic_resistance) }
                dt { "dmg / range" } dd { (c.damage) " / " (c.attack_range) }
                dt { "move" } dd { (c.move_speed) }
                dt { "ai" } dd { (c.ai_type) }
                dt { "reward" } dd { (c.exp_reward) " xp · " (c.gold_reward) " g" }
            }
            @if !u.abilities.is_empty() {
                div.tags {
                    @for a in &u.abilities { span.tag.ability { (a) } }
                }
            }
            (impl_block(u))
        }
    }
}

fn impl_block(u: &UnitEntry) -> Markup {
    if u.overrides.is_empty() && u.world_calls.is_empty() && u.source_file.is_none() {
        return html!{};
    }
    html! {
        details.impl-block {
            summary {
                "impl (" (u.overrides.len()) " hooks, "
                (u.world_calls.len()) " world calls)"
                @if let Some(src) = &u.source_file { " · " span.sub { (src) } }
            }
            @if !u.overrides.is_empty() {
                p { strong { "overrides: " }
                    @for (i, h) in u.overrides.iter().enumerate() {
                        @if i > 0 { ", " }
                        code { (h) }
                    }
                }
            }
            @if !u.world_calls.is_empty() {
                p { strong { "world calls: " }
                    @for (i, h) in u.world_calls.iter().enumerate() {
                        @if i > 0 { ", " }
                        code { (h) }
                    }
                }
            }
        }
    }
}

fn section_abilities(c: &Catalog) -> Markup {
    html! {
        section #abilities {
            h2 { "Abilities (" (c.abilities.len()) ")" }
            @for a in &c.abilities {
                (ability_card(a))
            }
        }
    }
}

fn ability_card(a: &crate::lib::model::AbilityEntry) -> Markup {
    let name = a.def_json.get("name").and_then(|v| v.as_str()).unwrap_or(&a.id);
    let desc = a.def_json.get("description").and_then(|v| v.as_str()).unwrap_or("");
    let search = format!("{} {} ability", a.id, name);
    html! {
        div.card data-search=(search) {
            h3 { (name) " " span.sub { "(" (a.id) ")" } }
            @if !desc.is_empty() { p.sub { (desc) } }
            details { summary { "def json" }
                pre.mono { (serde_json::to_string_pretty(&a.def_json).unwrap_or_default()) }
            }
        }
    }
}
fn section_api(_c: &Catalog) -> Markup { html! { section #api { h2 { "Script API" } p { "(coming)" } } } }
fn section_stat_keys(_c: &Catalog) -> Markup { html! { section #stat-keys { h2 { "Stat Keys" } p { "(coming)" } } } }
fn section_coverage(_c: &Catalog) -> Markup { html! { section #coverage { h2 { "Coverage Matrix" } p { "(coming)" } } } }
