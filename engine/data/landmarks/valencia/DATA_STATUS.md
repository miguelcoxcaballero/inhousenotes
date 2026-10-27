# Valencia landmarks: data status

Generated 2026-10-27 from the fetch run (`engine/tools/landmarks/fetch.py`) and a hand review of every
landmark's overlay (`overlay.png`) and photo contact sheet (`photos/contact.jpg`). Raw downloads live in
`engine/assets/downloaded/landmarks/valencia/<slug>/` (gitignored); committed data is `<slug>/footprint.json`
and `<slug>/sources.toml`. The photo hand review is in `photo_review.toml` (read by fetch.py).
Landmarks with a `landmark.toml` belong to a modeler: fetch.py skips them and never rewrites their data.

## Summary

- Landmarks: 100; complete (plausible outline, Catastro parts, >=5 exterior photos): **97**; with gaps: 3 (estacio_joaquim_sorolla, torre_de_l_angel, trinquet_de_pelayo).
- Outlines: 150 polygons; OSM building:parts 793; Catastro BuildingParts 1190; context buildings 31489.
- Photos: 1543 downloaded, 1467 exterior (hand-reviewed; auto classification for the modeled landmarks); 92 landmarks reach >=10 exterior photos.
- Plans/drawings: 194 files (every landmark has at least the Catastro WMS cadastral map).
- Exterior counts include the official Catastro facade photo when it shows the building.

## Notes for consumers (Rust team)

- Frame `utm30n-local-v1`: rings are landmark-local (city frame minus `anchor`), open, any orientation.
- Photo `heading` is degrees from true north; grid azimuth in the city frame = heading - 1.67 deg (`lmcommon.CONVERGENCE_DEG`).
- `floors_above`/`floors_below` may be null; monument floor counts in Catastro are rough. Context includes 0-floor parts (patios, plazas over garages).
- Several polygons of one OSM relation share the same `id`.
- Commons camera locations are sometimes wrong; `selected`/`side` in sources.toml are still for modelers to set.

## Per landmark

| # | slug | outline (method) | area m2 | OSM parts | Catastro parts | photos | exterior | plans | status | gaps / notes |
|---|---|---|---:|---:|---:|---:|---:|---:|---|---|
| 1 | llotja_de_la_seda | osm:way/23377876 (wikidata) | 1445 | 4 | 3 | 16 | 16 (auto) | 2 | complete | modeled (landmark.toml), data owned by the modeler, not refetched |
| 2 | catedral_de_valencia | osm:way/135630267 (wikidata) | 5945 | 53 | 7 | 16 | 15 (auto) | 7 | complete | modeled (landmark.toml), data owned by the modeler, not refetched |
| 3 | torres_de_serrans | osm:way/9281502 (wikidata) | 1193 | 10 | 6 | 16 | 16 (auto) | 4 | complete | modeled (landmark.toml), data owned by the modeler, not refetched |
| 4 | palau_de_les_arts | osm:way/23433773 (wikidata) | 9899 | 0 | 87 | 17 | 17 (auto) | 1 | complete | only the Catastro map as plan; modeled (landmark.toml), data owned by the modeler, not refetched |
| 5 | mercat_central | osm:way/5007139 (wikidata) | 8058 | 3 | 4 | 17 | 16 (auto) | 2 | complete | modeled (landmark.toml), data owned by the modeler, not refetched |
| 6 | estacio_del_nord | osm:way/10153012 (wikidata) | 5522 | 10 | 15 | 16 | 16 (auto) | 4 | complete | modeled (landmark.toml), data owned by the modeler, not refetched |
| 7 | torres_de_quart | osm:way/13073736 (wikidata) | 642 | 8 | 8 | 16 | 16 (auto) | 3 | complete | modeled (landmark.toml), data owned by the modeler, not refetched |
| 8 | palau_del_marques_de_dos_aigues | osm:way/515082661 (wikidata) | 1668 | 0 | 11 | 16 | 16 (auto) | 2 | complete | modeled (landmark.toml), data owned by the modeler, not refetched |
| 9 | museu_de_les_ciencies | osm:relation/14693200,osm:relation/14693200 (wikidata) | 18063 | 0 | 45 | 16 | 16 (auto) | 1 | complete | only the Catastro map as plan; modeled (landmark.toml), data owned by the modeler, not refetched |
| 10 | hemisferic | osm:way/23433774 (wikidata) | 3278 | 0 | 12 | 16 | 9 (auto) | 1 | complete | 9 exterior photos (<10); only the Catastro map as plan; modeled (landmark.toml), data owned by the modeler, not refetched |
| 11 | estadi_de_mestalla | osm:relation/7886308,osm:way/1239685453,osm:way/1122832534 (wikidata-area) | 32792 | 0 | 17 | 11 | 11 | 1 | complete | only the Catastro map as plan; stadium relation + 2 ways (stands and outer concourse); Catastro has the stands |
| 12 | mercat_de_colom | osm:way/10120283 (wikidata) | 2730 | 0 | 3 | 16 | 11 (auto) | 2 | complete | modeled (landmark.toml), data owned by the modeler, not refetched |
| 13 | oceanografic | osm:way/23433781,osm:way/194346757,osm:way/23438995,... (wikidata-area) | 9283 | 0 | 9 | 13 | 13 | 1 | complete | only the Catastro map as plan; complex: 15 separate pavilions/tanks (OSM ways inside the Oceanogràfic area); Catastro matches the main ones |
| 14 | ajuntament_de_valencia | osm:relation/11174923 (wikidata) | 5665 | 52 | 36 | 17 | 17 | 2 | complete |  |
| 15 | palau_de_la_generalitat | osm:relation/11137438 (wikidata) | 992 | 3 | 3 | 16 | 16 | 3 | complete |  |
| 16 | placa_de_bous | osm:relation/12942182 (list) | 5302 | 64 | 28 | 17 | 17 (auto) | 3 | complete | modeled (landmark.toml), data owned by the modeler, not refetched |
| 17 | basilica_dels_desemparats | osm:way/135630265 (wikidata) | 983 | 7 | 3 | 16 | 16 | 2 | complete |  |
| 18 | sant_nicolau | osm:way/514601809 (wikidata) | 1207 | 1 | 8 | 17 | 17 | 3 | complete |  |
| 19 | museu_de_belles_arts | osm:relation/11026210 (wikidata) | 7361 | 20 | 21 | 16 | 16 | 4 | complete |  |
| 20 | agora | osm:way/37523471 (wikidata) | 4877 | 0 | 7 | 17 | 17 | 1 | complete | only the Catastro map as plan |
| 21 | santa_caterina | osm:way/135638445 (wikidata) | 1637 | 5 | 2 | 16 | 16 | 3 | complete |  |
| 22 | sants_joans | osm:way/183237709 (wikidata) | 1807 | 11 | 44 | 17 | 17 | 2 | complete |  |
| 23 | el_patriarca | osm:relation/18681030 (wikidata) | 3176 | 2 | 11 | 17 | 17 | 3 | complete |  |
| 24 | edifici_de_correus | osm:way/513696236 (wikidata) | 2361 | 9 | 10 | 16 | 16 | 1 | complete | only the Catastro map as plan |
| 25 | sant_miquel_dels_reis | osm:relation/2024883,osm:way/1247022587 (wikidata-area) | 6571 | 7 | 12 | 15 | 15 | 7 | complete |  |
| 26 | palau_de_benicarlo | osm:way/1286369610 (wikidata) | 733 | 0 | 3 | 17 | 17 | 2 | complete |  |
| 27 | la_nau | osm:relation/11164787 (wikidata) | 3612 | 6 | 5 | 14 | 14 | 2 | complete |  |
| 28 | palau_de_la_musica | osm:way/23167750 (wikidata) | 2616 | 0 | 4 | 17 | 17 | 1 | complete | only the Catastro map as plan |
| 29 | convent_de_sant_domenec | osm:relation/7293544,osm:way/627764876,osm:way/497484159,... (wikidata-area) | 6928 | 1 | 21 | 17 | 17 | 3 | complete | convent/Capitania complex: relation + 5 ways, two cloister holes |
| 30 | almodi | osm:way/227916537 (wikidata) | 842 | 0 | 1 | 17 | 17 | 1 | complete | only the Catastro map as plan |
| 31 | drassanes_del_grau | osm:way/460056120,osm:way/460056118,osm:way/460056119,... (list) | 3397 | 0 | 5 | 16 | 16 | 2 | complete | 5 parallel naves (ids from list.toml) |
| 32 | teatre_principal | osm:way/23377878 (wikidata) | 1868 | 7 | 6 | 16 | 16 | 1 | complete | only the Catastro map as plan |
| 33 | umbracle | osm:way/23433784 (wikidata) | 9708 | 0 | 5 | 15 | 15 | 1 | complete | only the Catastro map as plan |
| 34 | veles_e_vents | osm:way/23509879 (wikidata) | 3366 | 15 | 2 | 17 | 17 | 1 | complete | only the Catastro map as plan |
| 35 | ivam | osm:way/291006854 (name) | 4667 | 13 | 9 | 13 | 13 | 1 | complete | only the Catastro map as plan |
| 36 | convent_del_carme | osm:relation/8082637 (wikidata) | 4916 | 18 | 2 | 14 | 14 | 3 | complete |  |
| 37 | palau_de_justicia | osm:relation/18681032 (wikidata) | 2812 | 0 | 3 | 16 | 16 | 2 | complete |  |
| 38 | banys_de_l_almirall | osm:way/218841695 (wikidata) | 275 | 0 | 3 | 21 | 8 | 2 | complete | 8 exterior photos (<10) |
| 39 | sant_joan_de_l_hospital | osm:way/514000994 (wikidata) | 989 | 0 | 5 | 16 | 16 | 3 | complete |  |
| 40 | roig_arena | osm:way/1014385791 (name) | 16030 | 28 | 58 | 14 | 14 | 1 | complete | only the Catastro map as plan |
| 41 | edifici_del_rellotge | osm:relation/21033515 (wikidata) | 931 | 5 | 4 | 17 | 17 | 1 | complete | only the Catastro map as plan; outline with two inner rings (light wells) from OSM |
| 42 | palau_de_congressos | osm:way/23761809 (wikidata) | 8560 | 0 | 25 | 12 | 12 | 1 | complete | only the Catastro map as plan |
| 43 | finca_roja | osm:relation/10512385,osm:relation/10512381,osm:relation/10512382,... (wikidata-area) | 7377 | 140 | 105 | 18 | 18 | 2 | complete | residential block made of 14 outlines around a courtyard |
| 44 | esglesia_del_temple | osm:relation/11177870 (name) | 3017 | 0 | 8 | 16 | 16 | 2 | complete |  |
| 45 | sant_marti | osm:way/661320825 (wikidata) | 1294 | 0 | 2 | 17 | 17 | 2 | complete |  |
| 46 | casa_dels_dracs | osm:way/496338533 (nearest) | 564 | 0 | 9 | 18 | 18 | 2 | complete | nearest-building fallback (point outside the way), but the outline is the Casa dels Dracs corner block |
| 47 | monestir_de_la_trinitat | osm:relation/11289994 (wikidata) | 3449 | 15 | 18 | 17 | 17 | 2 | complete |  |
| 48 | ateneu_mercantil | osm:way/416210407 (name) | 1069 | 15 | 15 | 17 | 17 | 1 | complete | only the Catastro map as plan |
| 49 | escoles_pies | osm:way/16190266 (contains) | 2962 | 1 | 6 | 17 | 17 | 3 | complete | college + rotunda church; dome as OSM part |
| 50 | bombas_gens | osm:way/701982887,osm:way/1550827170,osm:way/701982886,... (wikidata-area) | 3948 | 10 | 21 | 16 | 16 | 1 | complete | only the Catastro map as plan; factory complex, 4 outlines |
| 51 | beneficencia | osm:relation/3865968 (wikidata) | 5956 | 6 | 12 | 17 | 17 | 1 | complete | only the Catastro map as plan |
| 52 | sant_esteve | osm:way/279478350 (wikidata) | 1168 | 0 | 3 | 17 | 17 | 1 | complete | only the Catastro map as plan |
| 53 | palau_de_cervello | osm:way/495233207 (wikidata) | 1785 | 0 | 7 | 14 | 14 | 2 | complete |  |
| 54 | palau_de_l_almirall | osm:way/218841696 (wikidata) | 1439 | 0 | 9 | 11 | 11 | 2 | complete |  |
| 55 | palau_de_l_exposicio | osm:way/150277194 (wikidata) | 1722 | 0 | 8 | 17 | 12 | 2 | complete |  |
| 56 | estadi_ciutat_de_valencia | osm:relation/12211578 (wikidata-area) | 15142 | 9 | 9 | 8 | 8 | 1 | complete | 8 exterior photos (<10); only the Catastro map as plan; stadium ring outline; Catastro parcel larger (includes surroundings) |
| 57 | sant_agusti | osm:way/239784943 (wikidata) | 1675 | 0 | 10 | 17 | 17 | 2 | complete |  |
| 58 | la_union_y_el_fenix | osm:way/409877331 (contains) | 891 | 28 | 21 | 16 | 16 | 1 | complete | only the Catastro map as plan; corner block, 28 OSM parts with levels |
| 59 | tabacalera | osm:relation/9603911 (wikidata) | 8754 | 3 | 5 | 15 | 15 | 2 | complete |  |
| 60 | edifici_rialto | osm:way/416210393 (wikidata) | 714 | 18 | 11 | 17 | 17 | 1 | complete | only the Catastro map as plan |
| 61 | palau_del_marques_de_campo | osm:relation/7286126 (wikidata) | 1343 | 0 | 7 | 21 | 11 | 2 | complete |  |
| 62 | estacio_joaquim_sorolla | osm:way/554720470 (wikidata) | 9686 | 9 | 9 | 8 | 4 | 1 | gap | only 4 exterior photos; only the Catastro map as plan; Commons (category + full-text search) is almost all trains, platforms and the concourse; the Catastro facade photo is the Estacio del Nord parcel |
| 63 | cases_xapa | osm:way/477950785,osm:way/477950786,osm:way/457343282,... (list) | 3243 | 0 | 38 | 18 | 18 | 7 | complete |  |
| 64 | torre_hilton | osm:way/23761819 (name) | 3775 | 29 | 29 | 17 | 17 | 1 | complete | only the Catastro map as plan |
| 65 | torre_de_franca | osm:way/298794518 (wikidata) | 1107 | 19 | 14 | 17 | 17 | 1 | complete | only the Catastro map as plan |
| 66 | sant_tomas_i_sant_felip_neri | osm:way/495038787 (wikidata) | 1257 | 0 | 4 | 16 | 16 | 2 | complete |  |
| 67 | alqueria_del_moro | osm:way/1115796656 (wikidata) | 430 | 0 | 2 | 10 | 10 | 1 | complete | only the Catastro map as plan |
| 68 | banc_de_valencia | osm:way/492626775 (wikidata) | 882 | 23 | 20 | 17 | 17 | 1 | complete | only the Catastro map as plan |
| 69 | tinglado_2_del_port | osm:way/10380370 (wikidata) | 5821 | 0 | 1 | 18 | 18 | 2 | complete |  |
| 70 | antic_hospital | osm:way/15218692 (list) | 1781 | 4 | 2 | 12 | 12 | 1 | complete | only the Catastro map as plan |
| 71 | sant_joan_de_la_creu | osm:way/513977884 (wikidata) | 1578 | 0 | 4 | 17 | 17 | 2 | complete |  |
| 72 | palau_dels_catala_de_valeriola | osm:relation/16754082 (wikidata) | 969 | 0 | 11 | 12 | 12 | 2 | complete |  |
| 73 | art_major_de_la_seda | osm:way/514894836 (wikidata) | 850 | 6 | 6 | 12 | 12 | 2 | complete |  |
| 74 | esglesia_del_salvador | osm:way/493974568 (name) | 827 | 0 | 3 | 17 | 17 | 2 | complete |  |
| 75 | mercat_d_abastos | osm:way/243447549 (wikidata) | 13964 | 25 | 37 | 15 | 15 | 2 | complete |  |
| 76 | sant_miquel_i_sant_sebastia | osm:way/497514270 (contains) | 1414 | 0 | 5 | 17 | 17 | 2 | complete |  |
| 77 | casa_museu_blasco_ibanez | osm:way/512676794 (wikidata) | 186 | 2 | 3 | 9 | 9 | 1 | complete | 9 exterior photos (<10); only the Catastro map as plan |
| 78 | portal_de_la_valldigna | osm:way/807324595 (wikidata) | 254 | 0 | 2 | 15 | 15 | 1 | complete | only the Catastro map as plan |
| 79 | casa_jueva | osm:way/478348143 (wikidata) | 392 | 0 | 15 | 17 | 17 | 2 | complete |  |
| 80 | preso_model | osm:way/892748298 (name) | 8152 | 5 | 5 | 17 | 17 | 2 | complete |  |
| 81 | sant_valer | osm:relation/9498243 (name) | 1083 | 5 | 5 | 15 | 15 | 2 | complete |  |
| 82 | teatre_olympia | osm:way/415633647 (contains) | 2626 | 31 | 24 | 6 | 6 | 1 | complete | 6 exterior photos (<10); only the Catastro map as plan; outline is the whole Edifici Olympia block that contains the theatre (31 OSM parts, 24 Catastro parts) |
| 83 | misericordia_de_campanar | osm:relation/7356910 (wikidata) | 1020 | 0 | 4 | 17 | 17 | 2 | complete |  |
| 84 | mercat_de_russafa | osm:way/12403536 (wikidata) | 4749 | 3 | 3 | 13 | 13 | 1 | complete | only the Catastro map as plan |
| 85 | banc_d_espanya | osm:way/416487480 (list) | 1587 | 0 | 11 | 14 | 14 | 1 | complete | only the Catastro map as plan; own outline way/416487480 (list.toml); was sharing the Banc de Valencia way |
| 86 | palau_de_la_batlia | osm:way/1303164127 (name) | 814 | 0 | 4 | 10 | 10 | 2 | complete | OSM outline covers only ~64% of the Catastro parts; palace with courtyard, check Catastro for the full parcel |
| 87 | palauet_d_aiora | osm:way/23413021 (wikidata) | 250 | 0 | 2 | 15 | 15 | 2 | complete |  |
| 88 | centre_d_art_hortensia_herrero | osm:relation/11179029 (contains) | 1069 | 0 | 14 | 11 | 11 | 1 | complete | only the Catastro map as plan; Palau de Valeriola relation with courtyard |
| 89 | almoina | osm:relation/11150998,osm:relation/11150998 (wikidata) | 663 | 6 | 4 | 23 | 13 | 7 | complete |  |
| 90 | alqueria_de_julia | osm:way/892377652 (wikidata) | 327 | 4 | 4 | 13 | 13 | 2 | complete |  |
| 91 | palau_arquebisbal | osm:relation/17642970 (wikidata) | 2179 | 1 | 11 | 17 | 17 | 1 | complete | only the Catastro map as plan |
| 92 | torre_de_l_angel | osm:way/820481595 (name) | 59 | 0 | 0 | 12 | 12 | 1 | gap | no Catastro BuildingPart; only the Catastro map as plan; small wall tower + wall segment (59 m2) on a vacant lot: Catastro has no BuildingPart for it |
| 93 | trinquet_de_pelayo | osm:way/497229157 (wikidata) | 968 | 0 | 3 | 20 | 0 | 1 | gap | only 0 exterior photos; only the Catastro map as plan; every Commons file (category + full-text search, ~40) is the court, stands, players or food; the street front on Carrer de Pelai has no photo and no Catastro facade photo |
| 94 | la_companyia | osm:way/216891044 (wikidata) | 1246 | 2 | 6 | 17 | 17 | 2 | complete |  |
| 95 | edifici_ferrer | osm:way/496518123 (contains) | 309 | 0 | 2 | 16 | 16 | 2 | complete |  |
| 96 | muvim | osm:way/15218690 (wikidata) | 2166 | 8 | 10 | 16 | 16 | 1 | complete | only the Catastro map as plan |
| 97 | casa_del_punt_de_ganxo | osm:way/809242074 (wikidata) | 315 | 0 | 1 | 16 | 16 | 1 | complete | only the Catastro map as plan |
| 98 | casa_ortega | osm:way/496729376 (contains) | 459 | 0 | 7 | 15 | 15 | 1 | complete | only the Catastro map as plan |
| 99 | llotja_de_peixcadors | osm:way/796340087 (nearest) | 981 | 4 | 4 | 17 | 17 | 1 | complete | only the Catastro map as plan; nearest-building fallback; long nave with two side wings, matches Catastro |
| 100 | barraca_genuina_pinedo | osm:way/1493492059 (contains) | 496 | 0 | 2 | 5 | 5 | 2 | complete | 5 exterior photos (<10) |

## Photo review notes

- **palau_de_les_arts**: 8 Museu Ciencies; 9,10,16 Hemisferic foreground
- **museu_de_les_ciencies**: those mainly show Hemisferic/Umbracle
- **estadi_de_mestalla**: bowl interiors 1,11,12,16 useful for stands/roof
- **oceanografic**: animals
- **basilica_dels_desemparats**: 1 cathedral cimborio
- **sant_nicolau**: 2 interior; 9,12 tile panels
- **agora**: all exterior
- **santa_caterina**: 9 roofs from inside a tower
- **edifici_de_correus**: 3 city hall
- **sant_miquel_dels_reis**: C Catastro facade is the neighbouring barracks/prison; 1,8,9 cloisters
- **palau_de_benicarlo**: 5 view from the building
- **la_nau**: 9 cloister
- **palau_de_la_musica**: 7 distant; 10,15 Turia colonnade not the building
- **convent_de_sant_domenec**: 16 silhouette
- **teatre_principal**: 2,7 rooftop views not of theatre
- **umbracle**: those show Hemisferic, not Umbracle
- **veles_e_vents**: C Catastro facade shows a different (port warehouse) building; 10 distant
- **ivam**: 7 is Mont de Pietat in Alcoi; 9,12 paintings
- **convent_del_carme**: 10 model
- **banys_de_l_almirall**: interiors; exterior = roofs 4,5,10,16 + portal 11,12; small building enclosed
- **sant_joan_de_l_hospital**: courtyards 3,11,15 still exterior of church
- **roig_arena**: 9 is Alqueria del Basket (neighbour)
- **esglesia_del_temple**: 16 plaque
- **sant_marti**: several sculpture closeups; 15 title says Sant Valero but shows Sant Marti portal
- **casa_dels_dracs**: 5-8 dragon closeups
- **ateneu_mercantil**: 5,16 roof finials closeups
- **escoles_pies**: 14 unknown title
- **sant_esteve**: several gargoyle closeups
- **palau_de_cervello**: 7 painting; 16 historic 1870
- **palau_de_l_almirall**: 1,11,13,14,16 courtyard
- **estadi_ciutat_de_valencia**: bowl interiors (stands useful for roof modeling)
- **sant_agusti**: 2 ivy closeup; 3 Santa Caterina? statue corner
- **la_union_y_el_fenix**: 8 historic 1930s
- **palau_del_marques_de_campo**: courtyard/stairs; 16 aerial
- **estacio_joaquim_sorolla**: tracks/trains/platforms; Catastro facade photo C shows Estacio del Nord (wrong parcel)
- **torre_hilton**: distant skyline views (km away)
- **torre_de_franca**: 8 marathon runners; several distant views
- **sant_tomas_i_sant_felip_neri**: street lighting details
- **alqueria_del_moro**: 10 closeup
- **tinglado_2_del_port**: reliefs/capitals closeups
- **antic_hospital**: archaeological garden columns; C catastro facade is a different (modern) building; 14,15 closeups
- **sant_joan_de_la_creu**: portal closeups; 16 historic 1870
- **palau_dels_catala_de_valeriola**: courtyard; 3 is a neighbour (Plaza de Nules); 13 aerial
- **art_major_de_la_seda**: museum objects
- **esglesia_del_salvador**: 9 aerial
- **casa_museu_blasco_ibanez**: 12 historic print; small category
- **portal_de_la_valldigna**: C Catastro facade is a neighbouring house
- **sant_valer**: 11 Mercat de Russafa foreground
- **teatre_olympia**: interior auditorium; facade 1,2,7,8,12,C
- **palau_de_la_batlia**: 9,10 closeups; only 10 photos in category
- **palauet_d_aiora**: detail closeups
- **centre_d_art_hortensia_herrero**: only 4 photos selected (71 files classified interior: classifier issue)
- **almoina**: 7 shows cathedral; rest interior/artifacts
- **alqueria_de_julia**: 6,10,11,12 detail closeups
- **palau_arquebisbal**: 5,15 rooftop views from cathedral/Micalet (palace in the distance); 14 shows cathedral
- **torre_de_l_angel**: category has only 12 files
- **trinquet_de_pelayo**: ALL interior; no exterior photo; no Catastro facade photo
- **casa_del_punt_de_ganxo**: 14 is Palau de Berbedel/Museu de la Ciutat portal
- **casa_ortega**: many closeups; full facade 1,3,5,C only
- **llotja_de_peixcadors**: 7 neighbouring street
- **barraca_genuina_pinedo**: Commons category has only 4 photos
