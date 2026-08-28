# Auditoria da via de metadados — runlists NTFS órfãs

**Contexto.** O prompt afirma três falhas na etapa de sistema de arquivos e pede que a Fase 0
(medição de sobrevivência) preceda qualquer implementação. Esta auditoria mede tudo que podia ser
medido sem ler a mídia — manifestos de corridas anteriores, `/sys`, e o código — refuta duas
premissas do prompt, e **para antes de implementar**: os denominadores da Fase 0 não existem como
contadores, e obtê-los custa uma passagem de disco que ainda não foi gasta.

Nada aqui foi lido da mídia. Toda medição vem de `~/Imagens/argos-*/manifest.json`, de
`/sys/block`, e de leitura de código com âncora.

Etiquetas: `MEDIDO` (número deste repositório ou dos manifestos gravados), `INFERIDO` (premissas
nomeadas), `HIPÓTESE` (não medido), `REFUTADO`. Toda afirmação sobre código traz `arquivo.rs:linha`.

---

## 1. Veredito

1. **A mídia está conectada** (`ata-ST1000DM003-1CH162_S1DAZD8K` → `/dev/sdc`, 931,5 G), então a
   Onda 0 pode rodar. Duas condições a respeitar antes: **três partições dela estão montadas**
   (`sdc1` vfat, `sdc3` btrfs, `sdc5` btrfs, em `/run/media/breno/`) e precisam ser desmontadas — o
   kernel escreve num btrfs montado, e isso é escrita na mídia de origem; e **`acquire` da imagem de
   1 TB é impossível hoje**: o único destino com espaço (874 GB livres) é `sdc5`, o próprio disco de
   origem. `/home` tem 181 GB. `MEDIDO`
2. **Mas a parte decisiva da Fase 0 já está respondida offline**, no manifesto da corrida dirigida
   de 677 GiB (`~/Imagens/argos-ntfs/manifest.json`), que rodou sobre o volume NTFS residual **com
   geometria confirmada** — o caso que a inferência quer alcançar, já resolvido.
3. `MEDIDO` Naquela corrida a via de metadados produziu **111 recuperações**: **58 residentes**
   (conteúdo dentro do próprio registro, geometria nenhuma necessária) e **53 resolvidas por
   runlist**. Das 53: 45 em `fs-metadata`, 8 rebaixadas, **5 acima de 300 px**, **7 escritas**.
4. `MEDIDO` **Distribuição de trechos por runlist recuperada: 1 → 22, 2 → 30, 3 → 1.** É a resposta
   com dado real à pergunta de quantos fragmentos as fotos têm nesta mídia — e **57 % das
   recuperações por runlist são fragmentadas**, contra **101 de 100.535 (0,1 %)** da saída de
   carving na mesma corrida.
5. `INFERIDO` (de 3 + 4) A via **existe e é a única que recupera fragmentado a taxa**, mas seu teto
   absoluto nesta mídia é baixo: geometria inferida não pode superar geometria confirmada, e
   geometria confirmada rendeu 5 itens acima do piso em 630 GiB.
6. **Pico de deleção em lote: não observável.** `deleted_unix` ausente nas 111; `journal_deletions:
   0`. Há picos de *criação* (31 registros em 60 s, 25 em 3 s), que são cache de navegador sendo
   povoado, não uma deleção. `MEDIDO`
7. **Vale implementar a Fase 1 agora? Não.** Falta o numerador: nenhum contador diz quantos
   registros órfãos foram lidos, quantos validaram, quantos tinham `$DATA` não-residente. A Onda 0
   é instrumentação, custa uma passagem dirigida, e é o portão.
8. **Falha 3 do prompt: `REFUTADO`.** O `$UsnJrnl` **é** chamado em produção — `filesystem.rs:153`,
   ligado desde `OPEN-WORK §2.7`. O defeito real é outro: ele devolveu **zero** eventos em mídia
   real, com o nome do stream declarado não verificado (`ntfs.rs:224-228`).
9. **Falha 2 é real mas não envenena a Fase 1 pelo caminho que o prompt teme.** Volumes ext4
   fantasmas são filtrados antes da atribuição (`filesystem.rs:75`). Quem envenena é uma âncora
   **NTFS** fantasma, e há **nove delas medidas** neste disco (§6).

---

## 2. Medição de sobrevivência (Fase 0)

**Método.** Análise offline dos manifestos gravados em `~/Imagens/argos-*/manifest.json`. Residente
vs. não-residente separado pelo invariante do código: um `$DATA` residente resolve para
`record_at + at` (`ntfs.rs:825-828`), logo `0 ≤ extents[0].offset − source_object < 1024`; qualquer
outra coisa veio de runlist. Corrida analisada: `argos-ntfs`, 677.380.841.472 B a partir de
300,653 GiB, estado `cancelled` (cancelou na triagem, depois de `filesystem` terminar).

| Pergunta da Fase 0 | Resposta | Estado |
| --- | --- | --- |
| (1) regiões `FILE` órfãs / quantas validam | **não instrumentado** — só existe `unattributed_residue` (`finding.rs:242`), que conta regiões *não atribuídas*, e foi 0 nesta corrida | lacuna |
| (2) com `$DATA` não-residente e runlist decodificável | **≥ 53** (piso: tantas produziram achado). Numerador real desconhecido | parcial |
| (3) trechos por runlist | **1 → 22, 2 → 30, 3 → 1** (n=53) | `MEDIDO` |
| (4) timestamps / pico de lote | 50 instantes distintos em 111; maior balde de 60 s = **31**; balde de 3 s = **25**. Todos `$STANDARD_INFORMATION.created`. **Nenhum instante de deleção**: `deleted_unix` ausente em 111/111 | `MEDIDO` |
| (5) nome com extensão de imagem | 58 de 111 terminam em `.png`; os demais são nomes de cache (SHA-1 hex do CryptnetUrlCache), `edb0021B.log`, `.sqlite-shm`, `CiPT0000.001` | `MEDIDO` |
| (6) residentes | **58 de 111 (52 %)** — todos PNG, 162–614 B, **todos** rebaixados a `partial-or-thumbnail`, nenhum ≥ 300 px | `MEDIDO` |

**O que estes números forçam.** `INFERIDO` A metade residente da colheita são ícones e miniaturas de
cache: ela já funciona e não depende de geometria nenhuma. A metade por runlist é pequena e quase
toda pequena em bytes (mediana 8 KiB, máximo 1,25 MiB). O que a via entrega de valioso é
**fragmentação**, não volume.

**A pergunta (2) permanece aberta pelo lado que importa** — o denominador. 1.512 regiões órfãs
(corrida de 1 TB) e 12.125 (outra) são contagens de *regiões*, não de registros: `push_record_region`
(`residue.rs:210-219`) funde registros adjacentes numa faixa só. Uma região de 1.512 pode conter um
registro ou dez mil.

---

## 3. Onde a runlist morre hoje

O caminho, com âncoras:

1. `residue.rs:231` — `anchor_at` reconhece `FILE` por assinatura **mais** `is_plausible_record`
   (fixups verificados). Vira `Anchor::NtfsRecord`.
2. `residue.rs:190` → `push_record_region` (`residue.rs:210`) — vira `ByteRange` de 1024 B, fundida
   com a anterior se adjacente. Teto `MAX_RECORD_REGIONS = 65_536` (`residue.rs:48`).
3. `filesystem.rs:77-101` — o descarte. Duas portas em série:
   - `covers(volume.range, *region)` (`pipeline.rs:756`) sobre **apenas volumes NTFS**
     (`filesystem.rs:75`), **primeiro que casa vence, sem desempate** (`filesystem.rs:83`);
   - o volume precisa ter geometria confirmada com `volume_offset == volume.range.start`
     (`filesystem.rs:85-88`).
4. Falhou qualquer uma → `report.unattributed_residue += 1` (`filesystem.rs:90`) e
   `ntfs::orphan_records` (`ntfs.rs:599`) lê o registro para nome/tamanho/datas.

**Aqui está o defeito preciso, e é mais estreito e mais grave do que "descarta a runlist":**

`Record::into_lost_file` (`ntfs.rs:854-871`) **colapsa a runlist**:

```rust
RecordData::Runs { runs, .. } => (
    runs.iter().find_map(|run| run.lcn),                    // só o primeiro LCN
    runs.iter().fold(0, |s, r| s.saturating_add(r.clusters)) // só o total
),
RecordData::None | RecordData::Resident { .. } => (None, 0),
```

O mapa é decodificado corretamente por `decode_runs` (`ntfs.rs:1010`) e então **jogado fora dentro
do mesmo registro que sobreviveu**: sobram `first_lcn` e a soma de clusters (`ntfs.rs:568-572`). Não
há como reconstruir os trechos 2..n depois. Sem isso, a votação da Fase 1 só pode testar o primeiro
trecho de cada arquivo — perde justamente o poder discriminante que vem de N trechos concordarem.

**Discrepância documento/código a corrigir junto** (`M-CANONICAL-DOCS`): `finding.rs:252-255` afirma
que cada `LostFile` "carrega sua lista de runs nas unidades do próprio volume". Não carrega.

**Três perdas adicionais no mesmo caminho:**

| # | Perda | Âncora |
| --- | --- | --- |
| a | `orphan_scan`/`orphan_records` **nunca seguem `$ATTRIBUTE_LIST`** — só o walk do `$MFT` vivo absorve extensões (`ntfs.rs:198-205`). O órfão mais fragmentado trunca no que coube no registro base, vira `size > recovered` e é rebaixado. | `ntfs.rs:634-668`, `ntfs.rs:599-621` |
| b | Runlist com > `MAX_RUNS = 4096` trechos faz `decode_runs` devolver `None`, e o `?` em `ntfs.rs:762` **reprova o registro inteiro** em vez de truncar a lista. | `ntfs.rs:1047`, `ntfs.rs:762` |
| d | **O `$MFT` de um volume localizado é ele próprio uma região de registros `FILE`**, então todo registro dele é resolvido duas vezes: uma pelo walk do `$MFT` (`filesystem.rs:64`) e outra pelo `orphan_scan` sobre a região que o contém (`filesystem.rs:102`). Quando as duas confirmam, `consolidate` funde os dois achados e a duplicação é invisível; quando nenhuma confirma, o custo é pago duas vezes e some. Achado novo, exposto pelo contador da Onda 0 e cristalizado em `a_metadata_claim_the_medium_contradicts_is_counted_rather_than_reported` (`argos_engine/tests/pipeline.rs`). | `filesystem.rs:64`, `filesystem.rs:102` |
| c | Registros órfãos com o bit *in-use* **ligado** são ignorados (`ntfs.rs:661-664`, `ntfs.rs:612-615`). Num sistema de arquivos que não existe mais, "em uso" não significa nada: são arquivos igualmente perdidos. Teste que cristaliza a regra: `recovery.rs:1219`. | `ntfs.rs:659-664` |

`HIPÓTESE` (c) pode ser a maior perda de recall isolada do caminho órfão, e é a mais barata de
medir: um contador. Não proponho mudar o comportamento antes do número.

**Um quarto descarte, silencioso e sem contador:** `finding_from` (`filesystem.rs:383-386`) devolve
`None` quando não há extents ou quando o primeiro extent não começa com assinatura. Isso é o portão
de confirmação funcionando — mas ninguém conta quantos caem nele, e é exatamente esse número que
distingue "os clusters foram sobrescritos" de "a geometria está errada".

---

## 4. Fixups

**Aplicados, em todo caminho.** `MEDIDO` por leitura de código:

| Caminho | Sítio | Função |
| --- | --- | --- |
| Qualquer registro `FILE` | `ntfs.rs:699` (`Record::parse`, sobre cópia `raw.to_vec()`) | `apply_fixups` |
| `$MFT` registro 0 | `ntfs.rs:175`, `ntfs.rs:431` | idem |
| Walk do `$MFT` | `ntfs.rs:191`, `ntfs.rs:477` | idem |
| Registros de extensão | `ntfs.rs:810` | idem |
| Registros órfãos | `ntfs.rs:613`, `ntfs.rs:660` | idem |
| `$UsnJrnl:$J` (stream nomeado) | `ntfs.rs:336` | idem |
| `INDX` | `ntfs.rs:1110` | idem |
| Teste de âncora da varredura | `ntfs.rs:541` (`is_plausible_record`) | `fixups_verify` (só leitura, `M-MEM-REUSE`) |

Implementação: `apply_fixups` `ntfs.rs:953-965`, `fixups_verify` `ntfs.rs:983-997`, `fixup_header`
`ntfs.rs:968-975`. A varredura **exige** que o USA verifique antes de aceitar uma região
(`residue.rs:231`), então uma runlist lida a partir de região órfã já passou pelo fixup.

**Dois achados:**

- **`$MFTMirr` não existe no código.** Grep por `MFTMirr` na árvore inteira: zero ocorrências. O
  registro 1 nunca é usado como segunda chance nem para geometria nem para reparo. É lacuna, não bug.
- **Defeito latente, severidade alta em mídia 4Kn:** ambas as funções codificam o passo de setor
  como `512` literal (`ntfs.rs:958`, `ntfs.rs:989`), enquanto `from_boot_sector` aceita
  `bytes_per_sector` em `256..=4096` (`ntfs.rs:107-110`). Num volume NTFS 4Kn o USA tem passo 4096,
  `fixups_verify` compara os dois bytes errados, devolve `None`, e **todo registro daquele volume é
  rejeitado em silêncio** — inclusive via `has_mft`, o que faz `locate` descartar o volume inteiro.
  O disco desta auditoria é 512e, então isto não explica nenhum número medido; explica um zero
  futuro em disco 4Kn. Teste que falta: um fixture com `bytes_per_sector = 4096`.

**O teste pedido pelo prompt — registro cuja runlist cruze fronteira de setor — não existe.** O
fixture de conteúdo residente (`fixture.rs:562-596`) coloca a carga em 272..472, que nunca cruza
510. Isso esconde um bug real, descrito em §8.

---

## 5. Geometria inferida

**Estado: não implementado.** `OPEN-WORK §3.8` (`docs/OPEN-WORK.md:311-330`) descreve a ideia e
declara que não está feita. Nada no código infere geometria.

**Sinais disponíveis, e o que o código já tem de cada:**

| Sinal | Onde já é lido | Falta |
| --- | --- | --- |
| número do próprio registro (offset 44) | `ntfs.rs:718` — lido, usado só para desempatar `$ATTRIBUTE_LIST` | não é exposto em `LostFile` |
| passo de 1024 B entre registros | `DEFAULT_RECORD_SIZE` `ntfs.rs:56`, usado como constante | nunca derivado de uma corrida de registros consecutivos |
| runlist completa | `decode_runs` `ntfs.rs:1010` | **colapsada** em `into_lost_file` (§3) |
| tamanho declarado em bytes | `LostFile::size` `ntfs.rs:565` | já disponível |
| registro 0 / `$MFTMirr` como fonte direta | — | inexistente (§4) |

**Procedimento de votação proposto** (Onda 3, só se o portão abrir):

1. Amostrar registros órfãos com `$DATA` não-residente e runlist completa numa região.
2. Para cada `cluster_bytes ∈ {512, 1K, 2K, 4K, 8K, 16K, 32K, 64K}` — **4 KiB testado primeiro** — e
   cada início de volume candidato (derivado do número do registro e do passo observado):
   resolver o **primeiro** extent de cada registro e contar quantos carregam assinatura reconhecida
   por `argos_carve::identify` (`argos_carve/src/lib.rs:166`).
3. Verificação cruzada barata e independente, que o código já tem material para fazer:
   `clusters × cluster_bytes` deve cobrir `size` e não excedê-lo em mais de um cluster. O teste
   `recovery.rs:366-408` já afirma exatamente esse invariante para um `LostFile`.

**Limiar, declarado antes de medir:**

> Uma geometria candidata só é aceita se, sobre uma amostra de **≥ 200 registros com runlist
> não-residente**, sua taxa de acerto de assinatura for **≥ 5 % em absoluto** *e* **≥ 20× a taxa do
> melhor candidato rejeitado**. Amostra < 200 registros: nenhuma geometria é inferida,
> independentemente da taxa. Nenhum candidato acima do limiar: **nenhum volume inferido e nenhum
> extent emitido.**

O tamanho da amostra e a taxa de acerto são reportados junto com cada volume inferido, no manifesto.

**Resultado na mídia real: não medido.** A mídia não está conectada. Sem isso a Fase 1 não começa —
é o critério de rejeição do próprio prompt e concordo com ele.

**Nível de confiança — proposta.** Não criar variante em `Confidence`. O enum é ordenado por `Ord`
derivado (`argos_core/src/lib.rs:159-188`) e sua doc diz que uma tier nova é mudança do modelo de
recuperação; inserir uma variante reordena todas as comparações e as duas asserções de ordem
(`argos_core/tests/geometry.rs:41-48`, `argos_engine/tests/graft.rs:109-111`). Em vez disso:

- campo novo `geometry: GeometryProvenance` em `Finding` e no registro do manifesto, com
  `BootSector` ou `Inferred { cluster_bytes, sample, hits }`;
- a tier continua decidida **só** pela confirmação dupla, como hoje (`filesystem.rs:398-408`) — a
  inferência nunca a levanta nem a rebaixa;
- console e manifesto imprimem a proveniência em todo artefato inferido.

Isso satisfaz "sem elevar tier e sem esconder a diferença" sem mexer na escada.

---

## 6. Precisão do localizador de volumes

**O detector ext4** (`ext4.rs:133-167`) valida cinco predicados: magic `0xEF53` no offset 56 do
superbloco (= 0x438 do volume), `s_log_block_size ≤ 6`, `s_blocks_per_group ≠ 0`,
`s_inodes_per_group ≠ 0`, `s_inode_size` potência de dois em 128..4096.

**Não valida, e deveria:** `s_checksum` (0x3FC) e `s_checksum_type` — **nunca lidos em lugar nenhum
da crate**; `s_uuid` (104) — nunca lido, então duas cópias do mesmo superbloco não são reconhecíveis
como o mesmo volume; `s_inodes_count` (0); `s_blocks_count_hi` (336); coerência entre
`s_blocks_count`, `s_blocks_per_group` e a contagem de grupos; e sobretudo **nenhuma confirmação
contra a mídia** — nada lê a tabela de inodes do grupo 0, que é o análogo exato do que
`ntfs::locate` faz com o `$MFT` (`ntfs.rs:277-323`).

**Custo.** `MEDIDO` `OPEN-WORK:40-46`: estágio `filesystem` 32 min abrindo 15.186 volumes; cada
ext4 falso custa um `Ext4::open` mais uma caminhada de journal (`filesystem.rs:325-328`).
Ineficiência adicional: `scan_window:184` chama `anchor_at` → `volume_at`, e em caso de acerto chama
`volume_at` **de novo** em `residue.rs:186` — a cadeia inteira, incluindo Fletcher-64 do APFS e
crc32c do btrfs, roda duas vezes por acerto.

**Onde os falsos estão** — `MEDIDO`, do manifesto de 1 TB, offline:

| | |
| --- | --- |
| ext4 "volumes" | 15.157, **todos** `allocation_bytes = 4096`, **todos** `origin: residual` |
| comprimento truncado no fim da mídia | 11.559 de 15.157 (o campo `s_blocks_count_lo` reivindica além do disco) |
| baldes de 1 GiB tocados | **21** de 931 |
| três baldes mais densos | 464 GiB → 7.850; 47 GiB → 3.594; 510 GiB → 3.181 |
| aglomerado de 47 GiB | 3.594 acertos, **um único comprimento** (101.860.769.792 B) → mesmo superbloco repetido |
| aglomerados de 464 / 510 GiB | **um comprimento distinto por acerto** → `s_blocks_count_lo` varia a cada acerto |
| passo mediano dentro de um aglomerado | ~68 KiB (mínimo 12 KiB) |

`INFERIDO` São duas populações diferentes: uma é um superbloco real replicado (backups/journal), a
outra é conteúdo de arquivo que satisfaz cinco predicados fracos. Nenhuma é ruído aleatório —
para bytes aleatórios a probabilidade conjunta é ~2,5·10⁻¹⁴ por setor, ou ~5·10⁻⁵ acertos no disco
inteiro. **A causa exata é determinável com uma leitura dirigida de 15.157 × 1 KiB ≈ 15 MB**, nos
offsets que o manifesto já grava — segundos de disco, não uma passagem.

### Um volume fantasma pode envenenar a inferência?

**Pelo caminho ext4: não.** `filesystem.rs:71-76` filtra `kind == FsKind::Ntfs` antes de qualquer
atribuição, e `filesystem.rs:85-88` exige geometria confirmada. Um ext4 fantasma não chega lá.
`REFUTADO` para o cenário como o prompt o descreve.

**Pelo caminho NTFS: sim, e está medido acontecendo.** `filesystem.rs:83` toma **o primeiro volume
que cobre a região**, sobre uma lista ordenada por `(start, len)` (`filesystem.rs:227`). Do
manifesto de 1 TB, nove âncoras NTFS ordenam antes do volume residual real (300,653 GiB, 630,858 GiB,
cluster 4096) e alcançam dentro dele:

| início da âncora | comprimento reivindicado | sombreia do volume real | erro de offset |
| --- | --- | --- | --- |
| 157,975 GiB | 300,311 GiB | 157,633 GiB | 142,678 GiB |
| 158,592 GiB | 300,311 GiB | 158,249 GiB | 142,061 GiB |
| 158,971 GiB | 300,311 GiB | 158,629 GiB | 141,682 GiB |
| 175,202 GiB | 300,311 GiB | 174,859 GiB | 125,452 GiB |
| 189,427 GiB | 300,311 GiB | 189,084 GiB | 111,226 GiB |
| 238,615 GiB | 300,311 GiB | 238,273 GiB | 62,038 GiB |
| 276,729 GiB | 300,311 GiB | 276,387 GiB | 23,924 GiB |
| 280,093 GiB | 300,311 GiB | 279,750 GiB | 20,561 GiB |
| 300,653 GiB − 512 B | 300,311 GiB | 300,311 GiB | **512 B** |

`MEDIDO` (nove âncoras compartilham o comprimento reivindicado 322.455.993.856 B — são cópias do
mesmo setor de boot espalhadas pela superfície.)

**Severidade e mecanismo, com precisão.** O dano primário **não é fabricação, é perda silenciosa de
recall**: a região é atribuída à âncora errada, os extents resolvem para bytes arbitrários,
`argos_carve::identify` reprova, `finding_from` devolve `None` (`filesystem.rs:386`) — e o volume
**certo**, que cobre a mesma região, nunca é tentado. Nenhum contador registra o evento. A última
linha da tabela é o caso puro: 512 B de erro, e ela some com o volume real inteiro para todas as
regiões em 300,653–600,964 GiB.

A fabricação fica barrada pela confirmação dupla, e é isso que a mantém em severidade alta em vez de
máxima. Mas a barreira é probabilística: cluster errado com offset errado ocasionalmente aterrissa
num JPEG real, `identify` passa, e o veredito do estado do formato decide sozinho.

**Mitigante que já existe:** `confirm_ntfs` (`filesystem.rs:201-233`) exige que a geometria ponha um
registro `FILE` fixup-válido onde diz estar o `$MFT`. `INFERIDO` A corrida de 1 TB **antecede** esse
código — seu manifesto não tem sequer o campo `journal_deletions`, adicionado no mesmo commit
(364e08e, 2026-08-14) — então aquelas nove âncoras foram usadas **sem confirmação**. O risco residual
hoje é menor mas não é zero: esta mídia é densa em registros `FILE` genuínos, e uma âncora
coincidente cujo `$MFT` implícito caia sobre qualquer um deles é confirmada com `cluster_bytes` e
`volume_offset` errados.

**Correção, e ela é a mesma máquina da Fase 1:** em vez do primeiro volume que cobre, tentar **todos**
os volumes que cobrem e ficar com aquele cujo rendimento confirmado for maior. Votação por
rendimento resolve o sombreamento e a inferência com um mecanismo só.

### "Nenhum volume atual" — bug ou semântica?

`mark_current` (`residue.rs:110-119`) exige **igualdade exata** de `range.start` com uma entrada da
tabela de partições, e é chamado em `filesystem.rs:41` depois de `part::scan`.

**Resolvido: não é bug.** `MEDIDO`, com o disco agora conectado. Tabela de partições atual (lida de
`/sys/block/sdc/*/start`, sem tocar o dispositivo) contra o que o manifesto reportou:

| partição | início (B) | tipo vivo | volume reportado ali |
| --- | --- | --- | --- |
| sdc4 | 1.048.576 | — (bios_grub) | **nenhum** |
| sdc1 | 2.097.152 | vfat | **nenhum** |
| sdc2 | 502.267.904 | swap | **nenhum** |
| sdc3 | 4.502.585.344 | **btrfs** | **nenhum** |
| sdc5 | 74.503.421.952 | **btrfs** | **nenhum** |

`mark_current` não tinha o que casar: **nenhum dos 15.186 volumes localizados fica num início de
partição atual** (o menor offset reportado é 136.313.856). A causa é a cobertura de detectores
daquela build, não a igualdade exata: o detector btrfs entrou em f4d0392 (2026-08-25), **depois**
daquela corrida, e as duas partições vivas de dados são btrfs; a ESP `sdc1` é vfat e o validador FAT
exige o marcador FAT32 `u16@17 == 0` (`fat.rs:139`), rejeitando FAT16. Uma corrida com a build atual
deve marcar `sdc3` e `sdc5` como `Origin::Current` — e isso é um item de aceitação da Onda 1, não
uma correção a fazer.

`MEDIDO` Nota que reposiciona tudo: o volume NTFS residual (322.824.044.544) fica **inteiramente
dentro de `sdc5`**, a partição btrfs viva de 862 GB. O antigo Windows não foi apagado por uma
partição nova em cima dele; ele está sob um sistema de arquivos em uso.

---

## 7. `$UsnJrnl`

**Premissa do prompt refutada.** O leitor **é** chamado em produção:
`filesystem.rs:123` → `name_from_change_journal` (`filesystem.rs:139-173`) →
`geometry.change_journal(view)` (`filesystem.rs:153`) → `Ntfs::change_journal` (`ntfs.rs:422`) →
`usn_deletions` (`ntfs.rs:1180`). Registrado em `OPEN-WORK §2.7`.

**Quais regiões alimentam a leitura.** `find_journal` (`ntfs.rs:462-488`) caminha o `$MFT` inteiro
procurando um registro chamado `$UsnJrnl`, depois extrai o stream nomeado `$J` via
`named_stream_extents` (`ntfs.rs:331-375`) — que decodifica a runlist própria do `$J`, com fixups
aplicados. `read_journal_tail` (`ntfs.rs:494-525`) lê os últimos 64 MiB (`MAX_JOURNAL_BYTES`,
`ntfs.rs:237`). Só volumes com geometria **confirmada** (`filesystem.rs:149`).

**O que entrega.** `UsnDeletion { name, mft_record, timestamp }` (`ntfs.rs:1156-1163`), casado ao
achado por `source_object` — o offset absoluto do registro (`ntfs.rs:446-453`). Só nomeia e data;
nunca cria extent, nunca levanta tier (`filesystem.rs:160-169`). O prompt está certo sobre isso: não
pode fabricar.

**O que o pico de timestamps revelou: nada, porque não houve journal.** `MEDIDO`
`journal_deletions: 0` em duas corridas independentes (`argos-lote73`, `argos-ntfs`), a segunda
sobre 630 GiB de volume NTFS residual com geometria confirmada. `deleted_unix` ausente nas 111
recuperações por metadados.

**Duas explicações, e o projeto nunca rodou o teste que as separa** (`ntfs.rs:224-228`, também em
`OPEN-WORK §3.9a`): ou o journal não sobreviveu, ou a constante `JOURNAL_STREAM = "$J"` e/ou o
layout `USN_RECORD_V2` estão errados — todo fixture escreve o que a constante diz, então os testes
provam que leitor e fixture concordam e nada mais. **Custo do discriminante: uma leitura dirigida
ao `$MFT` do volume confirmado (offset conhecido), procurando o registro `$UsnJrnl` e listando os
nomes de stream que ele realmente tem.** Minutos, sem passagem completa.

**Guarda que também vale medir:** `filesystem.rs:146-148` retorna cedo se nenhum achado tem
`source_object`. Com 111 achados carregando `source_object`, não foi essa a causa nesta corrida.

**Custo de "ligar": zero — já está ligado.** O trabalho real é validar o layout contra mídia real e
registrar o resultado, incluindo se for negativo.

---

## 8. Verificação

Os quatro pontos do prompt, contra o que o código faz hoje (`finding_from`, `filesystem.rs:378-417`):

| Ponto | Implementado | Âncora |
| --- | --- | --- |
| assinatura no início do primeiro extent | sim — `identify` sobre `MAX_SIGNATURE_BYTES` lidos em `extents[0]`; falha ⇒ achado descartado | `filesystem.rs:383-386` |
| extents concatenados decodificam ponta a ponta | sim — `Assembled` + `argos_carve::validate`, a mesma máquina de estados do carving | `filesystem.rs:425-436` |
| comprimento bate com o declarado no atributo | sim — `file.size <= recovered` | `filesystem.rs:398` |
| gravado com hash | sim — SHA-256 em `output.rs:313-334`, com **re-verificação cruzada**: se o digest do que o sink recebeu diferir, a corrida inteira aborta (`output.rs:250-260`) | `output.rs` |

**Quantos passaram, na corrida de 677 GiB:** `MEDIDO` **45 dos 111** passaram os quatro
(`fs-metadata`); **66 falharam o segundo ou o terceiro** e foram reportados como
`partial-or-thumbnail` — o comportamento correto, e é honesto contá-los à parte.
Dos 45: **todos** vieram de runlist (nenhum residente passou).

**Assimetria a registrar:** para um candidato carveado a validação é portão de *existência*
(`carving.rs:159`); para um achado de metadados é portão de *tier* — reprovado ainda é emitido,
rebaixado (`filesystem.rs:404-408`). Isso é deliberado e documentado (`filesystem.rs:371-374`), e
concordo: o registro é evidência real de que o arquivo existiu.

**Bug de correção encontrado na verificação, e ele produz bytes errados:** para `$DATA` residente,
`data_extents` devolve o intervalo absoluto `record_at + at` (`ntfs.rs:825-828`). O parse trabalha
sobre uma **cópia com fixups aplicados** (`ntfs.rs:698-699`), mas o extent é lido **cru do disco**
depois, por `finding_from` (`filesystem.rs:385`) e por `output::emit`. Uma carga residente que
cruze uma fronteira de 512 B dentro do registro conterá **os dois bytes do USN** no lugar dos bytes
reais do arquivo. O fixture existente (`fixture.rs:562-596`) põe a carga em 272..472 e nunca cruza
510, então nenhum teste pega isso. Com 58 de 111 recuperações residentes nesta mídia, o caminho é
quente. `HIPÓTESE` quanto ao impacto observado; `MEDIDO` quanto ao código.

---

## 9. Plano

Cada onda tem predição pré-registrada, critério numérico e reversão. **Nenhuma onda depois da 0
começa antes dos números da 0.**

**Métrica de aceitação, escolhida antes de medir:** achados **multi-extent** (2+ extents). É onde a
via tem vantagem estrutural demonstrada — 30 de 53 recuperações por runlist são fragmentadas (57 %)
contra 101 de 100.535 do carving (0,1 %) — e é a única métrica cuja base não é pequena demais para
falsificar. Contagem total e itens acima de 300 px continuam reportados, como contexto.

**Pré-condições operacionais de toda corrida na mídia** (`MEDIDO`, hoje):
`umount /dev/sdc1 /dev/sdc3 /dev/sdc5` antes de qualquer leitura — um btrfs montado é escrito pelo
kernel, e a mídia de origem não pode ser escrita. **`acquire` está fora**: nenhum destino tem ~1 TB
livre fora do próprio `sdc5`. Logo a primeira passagem é um scan dirigido, e cada passagem conta.

### Onda 0 — instrumentar e preservar o mapa (portão) — **código pronto, medição pendente**

Nenhuma mudança de recuperação. Só medição e a preservação do dado que a Fase 1 precisará.

**Estado.** Implementada e verde: `cargo fmt --check`, `cargo clippy --workspace --all-targets
-D warnings` e `cargo test --workspace` passam. O snapshot de caracterização
(`crates/argos/tests/characterization/manifest.snapshot.json`) mudou **apenas** pelos três campos
novos de `coverage`; os nove artefatos são byte-idênticos, o que é a prova de que a instrumentação
não mexeu no que se recupera. **A aceitação continua pendente**: ela é a corrida na mídia, e essa
não foi feita.

**Arquivos:** `crates/argos_fs/src/ntfs.rs` (`LostFile`, `into_lost_file`),
`crates/argos_engine/src/finding.rs` (`ScanReport`),
`crates/argos_engine/src/pipeline/filesystem.rs` (contadores no laço de regiões),
`crates/argos_report/src/manifest.rs` + `crates/argos/src/scan.rs` (serialização),
`crates/argos/src/console.rs` (impressão).

1. `LostFile` passa a carregar a runlist inteira (`Box<[Run]>`, `Run` exposto) e o número do próprio
   registro (já lido em `ntfs.rs:718`). Isso **cumpre** o que `finding.rs:252-255` já afirma.
2. Contadores novos em `ScanReport`, todos preenchidos no laço `filesystem.rs:77-114` e dentro de
   `orphan_records`/`orphan_scan`: regiões atribuídas; registros lidos; registros que validam
   (magic + fixup); **com in-use ligado**; residentes; não-residentes com runlist decodificável;
   histograma de trechos por runlist; registros com nome de extensão de imagem; histograma de
   `$STANDARD_INFORMATION` em baldes de 60 s; e — o que falta hoje — **quantos achados
   `finding_from` descartou por falta de assinatura no primeiro extent**.
3. Nenhuma mudança em `covers`, em tiers, em limiares.

**Predição pré-registrada.** Sobre `--metadata-only --range 322824044544..` na mídia real:
`orphan_records_nonresident ≥ 53` (piso: tantas já produziram achado) e `≤ 5.000`. Abaixo de 53 a
instrumentação está errada. **Acima de 20.000, a hipótese central desta auditoria muda**: a perda
não é geometria, é o que acontece entre "registro lido" e "achado emitido", e a Onda 3 fica
suspensa até isso ser explicado.

**Aceitação.** A corrida reproduz **exatamente 111** achados de estágio `filesystem` — 45
`fs-metadata`, 66 `partial-or-thumbnail`, **31 multi-extent** (30 com 2 extents, 1 com 3) — e agora
reporta os seis números da Fase 0. Comparação por `sha256` contra
`~/Imagens/argos-ntfs/manifest.json`. Divergência em qualquer artefato invalida a onda: a
instrumentação não pode mudar o que é recuperado.
**Reversão:** os contadores são aditivos; remover é apagar campos do manifesto.

**Custo:** uma passagem dirigida de 630 GiB ≈ 1 h 20 m a 138 MB/s. Escolhida como primeira porque é
o **único intervalo com baseline** para validar a instrumentação contra.

### Onda 0b — três medições que não custam passagem de disco

Independentes entre si, executáveis assim que o disco aparecer:

- **Causa dos 15.157**: ler 1 KiB em cada offset ext4 gravado no manifesto (≈ 15 MB) e classificar.
- **`$UsnJrnl` real**: ler o `$MFT` do volume confirmado, achar o registro `$UsnJrnl`, listar os
  nomes de stream que ele tem de fato. Resolve `OPEN-WORK §3.9a`.
- ~~"Nenhum atual"~~ — **já resolvido** em §6 sem tocar o disco.

### Onda 1 — confirmar a âncora ext4 (`OPEN-WORK §3.6`)

Ler a tabela de inodes do grupo 0 e exigir inodes plausíveis, exatamente como `ntfs::locate` lê o
`$MFT`. Âncora não confirmada não é reportada. Arquivo: `crates/argos_fs/src/ext4.rs` (função de
confirmação nova), consumida em `crates/argos_engine/src/pipeline/filesystem.rs:201-233` ao lado de
`confirm_ntfs`. Somar os campos baratos que faltam ao validador (`s_uuid ≠ 0`, `s_inodes_count`
coerente, checksum quando `metadata_csum` estiver ligado).

**Predição.** `volumes` cai de 15.186 para **< 100** numa corrida de disco inteiro; a contagem de
âncoras NTFS permanece **exatamente 29**; o estágio `filesystem` cai de 32 min para **< 5 min**.
**Refutado se** a contagem NTFS mudar, ou se `volumes > 1.000`.
**Aceitação.** Fixture com superbloco solto e nenhum sistema de arquivos atrás dele não produz
volume; os testes de recuperação ext4 existentes ficam idênticos. Adicionalmente, com o detector
btrfs presente, **`sdc3` (4.502.585.344) e `sdc5` (74.503.421.952) aparecem com
`origin: current`** — é o teste que fecha §6 e prova que `mark_current` sempre funcionou.
**Reversão:** um commit; a confirmação é aditiva ao caminho de âncora.

### Onda 2 — matar o sombreamento

`filesystem.rs:83` deixa de tomar o primeiro volume que cobre. Tenta **cada** volume que cobre, na
ordem do menor para o maior, e fica com o que produzir achados confirmados; empate zero-a-zero não
emite nada. Mesma mudança em `name_from_index_slack` (`filesystem.rs:269-279`). Contador novo:
regiões em que mais de um volume cobria.

**Predição.** O número de achados de estágio `filesystem` **não diminui** em nenhuma corrida, e na
corrida de disco inteiro sobe em relação a uma corrida de controle. **Refutado se** cair.
**Reversão:** um commit; o comportamento antigo é o caso de um volume só.

### Onda 3 — geometria inferida (**só se o portão da Onda 0 abrir**)

Portão explícito: só começa se a Onda 0 mostrar **≥ 200 registros com `$DATA` não-residente e
runlist decodificável** em regiões que nenhum volume confirmado cobre. Abaixo disso a via não existe
nesta mídia e a auditoria encerra com essa conclusão.

Implementação em `crates/argos_fs/src/ntfs.rs` (uma função de votação sobre `&[LostFile]`,
sans-I/O exceto o teste de assinatura), consumida em `filesystem.rs` no ramo hoje ocupado por
`report.unattributed_residue += 1`. Limiar exatamente como declarado em §5. Proveniência exatamente
como proposto em §5 — sem variante nova em `Confidence`.

**Predição pré-registrada**, na métrica escolhida (multi-extent), com N e M a fixar a partir dos
denominadores da Onda 0 e **antes** de rodar: sobre o intervalo `0..322824044544`, achados de
estágio `filesystem` com 2+ extents saem de 0 para **≥ N**; abaixo de **M** a hipótese está refutada
e a Onda 3 é revertida inteira. Regra que fixa N e M sem espaço para ajuste posterior: **N = 10 % e
M = 2 %** da contagem de registros órfãos com `$DATA` não-residente e runlist decodificável que a
Onda 0 encontrar naquele intervalo. Se a Onda 0 encontrar menos de 200 desses registros, a Onda 3
não começa (portão acima).

**Fixture obrigatório**, que hoje não existe como builder nomeado: volume NTFS com **setor de boot
primário e cópia ambos zerados** — hoje recupera zero (`pipeline.rs:394-400` já monta esse caso à
mão para o teste de `lost_files`), depois deve recuperar o que recupera com eles intactos. A
composição existe em `crates/argos_fs/src/fixture.rs`; falta o builder.

### Fora de escopo, registrado

`orphan_scan` seguir `$ATTRIBUTE_LIST` (§3a), `MAX_RUNS` truncar em vez de reprovar (§3b), registros
órfãos in-use (§3c), passo de setor 4Kn nos fixups (§4), bytes residentes sem fixup (§8), `$MFTMirr`
(§4). Cada um é um defeito independente com correção pequena; nenhum depende da inferência. Ordená-los
depois dos números da Onda 0.

---

## 10. Limites honestos

- **Cluster sobrescrito não volta.** A runlist aponta para onde os bytes estavam; se a região foi
  reusada, ela aponta para o que está lá agora, e `identify` reprova em `filesystem.rs:386` — a
  verificação faz o certo, e o achado desaparece **sem contador**. Corrigir isso é item 2 da Onda 0.
- **Slot de MFT reusado apaga o registro.** Só sobrevive o que não foi reaproveitado. `INFERIDO`
  Registros de metadados são pequenos e vivem numa zona dedicada, reciclada a cada escrita de
  arquivo; clusters de dados de 1–5 MB reciclam noutro ritmo — a morte de um não prova a do outro,
  nos dois sentidos.
- **Arquivos sob Demanda do OneDrive: não distinguíveis hoje, e digo que não há.** O parser não lê
  `$REPARSE_POINT` (0xC0) — a lista de tipos de atributo em `ntfs.rs:35-38` tem
  `$STANDARD_INFORMATION`, `$ATTRIBUTE_LIST`, `$FILE_NAME` e `$DATA`, e mais nada. Um placeholder
  tem registro MFT e nenhum byte inteiro na mídia, e hoje entra no mesmo caminho de qualquer arquivo.
  A distinção é implementável (a tag de reparse point identifica o provedor) e não está feita.
- **Runlist esparsa: tratada.** `decode_runs` marca `off_size == 0` como `lcn: None`
  (`ntfs.rs:1027-1033`), e `data_extents` consome os offsets de arquivo do buraco sem emitir extent
  (`ntfs.rs:834-846`) — os trechos seguintes ficam atribuídos à parte certa do arquivo.
- **Runlist comprimida: fora de escopo, e o código não sabe disso.** O flag de compressão (offset 12
  do cabeçalho não-residente) e o tamanho da unidade de compressão (offset 34) **nunca são lidos** —
  só 8, 9/10, 32 e 48 (`ntfs.rs:758-764`). Um `$DATA` LZNT1 produz extents apontando para clusters
  comprimidos apresentados como conteúdo cru. Na prática `finding_from` os descarta, porque o
  primeiro cluster comprimido não carrega assinatura — mas o descarte é acidental, não projetado.
  Declarar fora de escopo **e ler o flag para rejeitar explicitamente** é uma linha de código.
- **Amostra pequena = geometria fraca.** O limiar de §5 recusa inferir abaixo de 200 registros, e a
  proveniência carrega amostra e taxa em todo artefato.

---

## 11. O que não foi medido

1. **Tudo que exige leitura da mídia.** Ela está conectada, mas nada foi lido dela nesta auditoria:
   toda medição aqui vem de manifestos gravados, de `/sys` e do código. Faltam os números da Fase 0
   sobre regiões órfãs, o teste de votação e a verificação do `$UsnJrnl` real.
2. **O denominador da pergunta (2):** quantos registros existem nas 1.512 (ou 12.125) regiões. Só
   existe a contagem de regiões, e regiões fundem registros adjacentes (`residue.rs:210-219`).
3. **A causa dos 15.157 falsos ext4.** Duas populações identificadas pela estrutura dos dados
   gravados; a causa exige os 15 MB de leitura dirigida da Onda 0b.
4. **Se as nove âncoras NTFS sombreadoras sobrevivem a `confirm_ntfs` hoje.** A corrida que as
   registrou antecede aquele código.
5. **Quantos achados `finding_from` descarta por assinatura ausente.** Não há contador — é a
   diferença entre "sobrescrito" e "geometria errada", e hoje o pipeline não a mede.
6. **`ceilings.detection`** não foi disparado na corrida de 1 TB (o manifesto lista só
   `"reassembly decode budget"`), então a varredura foi completa. `MEDIDO` — isso **refuta**
   a hipótese de que o ruído ext4 teria estourado `MAX_VOLUMES` e cegado trechos da superfície.
7. **O layout `USN_RECORD_V2` e o nome `$J`** contra mídia real (`ntfs.rs:224-228`).
8. **Custo do sweep por validador.** Não há benchmark de `residue::scan_window`/`volume_at`, e são
   ~2,1 milhões de invocações por GiB — o maior caminho quente não medido do workspace.
9. **Números do prompt que não localizei no repositório:** "1,25 %" de acerto de remontagem (o mais
   próximo é 3/254 = 1,18 % na corrida de 1 TB e 135/1876 = 7,2 % na de 677 GiB) e as "12.125"
   regiões órfãs. Reportados como vieram, não confirmados.

---

## 12. Perguntas

Duas já foram respondidas e saem da lista: a métrica de aceitação é **achados multi-extent**
(decidido), e `acquire` está fora por falta de destino — a primeira passagem é um scan dirigido
(determinado pela medição de espaço livre, não por preferência).

1. **Posso desmontar `sdc1`, `sdc3` e `sdc5` antes da corrida?** São montagens em
   `/run/media/breno/`; enquanto estiverem montadas, o kernel escreve na mídia de origem. É a única
   pré-condição bloqueante da Onda 0.
2. **Existe algum disco externo de ≥ 1 TB que possa receber uma imagem?** Muda a economia de todas
   as ondas seguintes: com imagem, cada re-medição custa zero passagem de disco em vez de 1 h 20 m.
3. **O intervalo `0..300,653 GiB` — a única faixa que nenhuma corrida com `confirm_ntfs` varreu — é
   onde o lote procurado deveria estar, ou ele estava dentro do volume NTFS residual?** Decide se a
   Onda 0 gasta a segunda passagem lá ou se a via já está medida onde importa.
4. **A proposta de proveniência (`GeometryProvenance` em `Finding`, sem variante nova em
   `Confidence`) está aceita**, ou prefere-se uma tier separada apesar do custo de reordenar a
   escada e as duas asserções de ordem?
5. **Os seis defeitos independentes de §9 "Fora de escopo" — em especial registros órfãos com o bit
   in-use ligado (§3c) e `$ATTRIBUTE_LIST` não seguido no caminho órfão (§3a) — entram na Onda 0
   como contadores, ou quer que já sejam corrigidos?** Meço antes de corrigir, salvo instrução em
   contrário.

---

## Verificação (como rodar cada onda)

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Testes que **devem** continuar passando sem alteração, porque são o contrato desta área:

- `crates/argos_fs/tests/recovery.rs:340` `ntfs_orphan_scan_finds_records_outside_any_mft`
- `crates/argos_fs/tests/recovery.rs:366` `an_orphaned_record_names_and_dates_its_file_without_any_volume`
- `crates/argos_fs/tests/recovery.rs:411` `a_record_truncated_at_any_boundary_is_never_misread`
- `crates/argos_engine/tests/pipeline.rs:380` `a_deleted_file_whose_volume_is_gone_is_still_named_and_dated`
- `crates/argos_carve/tests/recovery_rate.rs` — o piso de 87 %/87 %/25 % com **0 fabricados**

Na mídia (Onda 0), somente leitura, **e só depois de desmontar**:

```bash
umount /dev/sdc1 /dev/sdc3 /dev/sdc5          # o kernel escreve num btrfs montado
argos scan /dev/disk/by-id/ata-ST1000DM003-1CH162_S1DAZD8K \
  --metadata-only --range 322824044544.. --out <dir> --min-long-side 0
```

Aceitação: **111** achados de estágio `filesystem` — 45 `fs-metadata`, 66 `partial-or-thumbnail`,
**31 multi-extent** — byte-idênticos aos de `~/Imagens/argos-ntfs/manifest.json`, mais os seis
números da Fase 0 no manifesto. Comparação offline por `sha256` de cada artefato; qualquer
divergência reprova a onda.

**Proibições respeitadas em todas as ondas:** nenhum extent de geometria não confirmada pela votação
acima do limiar declarado; a confirmação dupla de `filesystem.rs:378-417` não é afrouxada; nenhuma
tier é elevada por inferência; nada é contado como recuperado sem os quatro pontos; nada fora da
etapa de sistema de arquivos é tocado; a mídia é aberta somente para leitura.

---

# Resultado da Onda 0 — a medição na mídia

Corrida de 2026-08-26, `/dev/disk/by-id/ata-ST1000DM003-1CH162_S1DAZD8K`, disco inteiro,
`--metadata-only --min-long-side 0 --no-triage`, 12 workers, estado `finished`.
**5 h 35 m**: varredura 2 h 00 m (139 MB/s), estágio `filesystem` **3 h 35 m**, relatório 6 s.
Sessão em `~/Imagens/argos-onda0`. Todos os números abaixo são `MEDIDO`.

## A predição pré-registrada está REFUTADA

> "`orphan_records_nonresident ≥ 53` e `≤ 5.000`. **Acima de 20.000, a hipótese central desta
> auditoria muda**: a perda não é geometria, é o que acontece entre 'registro lido' e 'achado
> emitido'."

**`non_resident = 342.514.`** Dezessete vezes o limiar de refutação, 68× o teto previsto. As
runlists não estão faltando: há um terço de milhão delas, decodificadas, nesta mídia.

## Os seis números da Fase 0

| | |
| --- | --- |
| regiões `FILE` órfãs | **42.204** — 30.079 atribuídas a volume confirmado, 12.125 não |
| registros lidos / íntegros | **3.036.191 / 3.036.190** (um único registro reprovou o fixup) |
| **em uso, e por isso não reportados** | **2.521.693 — 83 % de tudo que foi lido** |
| residentes (recuperáveis sem geometria) | **96.796** |
| **não-residentes com runlist decodificável** | **342.514** |
| com nome de extensão de imagem | **21.398** |
| trechos por runlist (1/2/3/4/5–8/9–16/17–64/65+) | **326.601 / 13.191 / 1.397 / 517 / 553 / 114 / 110 / 31** |
| pico de lote | **14.011 registros em 60 s** a partir de 2020-11-15 17:40:32 |

## O gargalo não é geometria — são os clusters

**71 % das regiões já tinham geometria confirmada** e tiveram suas runlists resolvidas contra ela.
O que aconteceu com elas:

| | |
| --- | --- |
| reivindicações de metadados emitidas | ~508.000 |
| **contraditadas pela mídia** (`metadata_unconfirmed`) | **499.788** |
| achados do estágio `filesystem` | 8.458 → **5.556 artefatos** (414 duplicatas) |
| **dos quais pela via NTFS** | **112** — 45 `fs-metadata`, 67 rebaixados |

A via de metadados propõe e a mídia recusa, na proporção de **~3.000 : 1**. Os mapas sobreviveram;
os clusters que eles apontam foram reusados. Note que 45 `fs-metadata` é **exatamente** o mesmo
número da corrida dirigida anterior: varrer o disco inteiro com a build atual não moveu a via NTFS
em um único artefato.

## Fase 1 (geometria inferida): CANCELADA, refutada pela população

O critério de aceitação era achados **multi-extent**, com N = 10 % e M (piso de refutação) = 2 % da
população não-residente das regiões não atribuídas.

| | |
| --- | --- |
| registros não atribuídos | 164.946, **todos abaixo de 300,653 GiB** (a faixa nunca varrida) |
| com runlist decodificável | 119.296 (72,3 %) → **N = 11.929, M = 2.385** |
| **com 2 ou mais trechos — o teto absoluto** | **1.300** |
| desses, nomeando uma imagem | **3** |
| desses, ≥ 100 KB | **1** |

**O teto do que é possível (1.300) está abaixo do piso de refutação (2.385)**, antes de ler um byte:
98,9 % das runlists órfãs têm um único trecho. Inferir geometria perfeitamente, em toda a faixa,
não pode atingir o critério. Esta é a conclusão que o prompt declarou aceitável — *"a via não paga"*
— e ela vem com a contagem.

## O que de fato voltou

5.556 artefatos, **17,3 MiB no total**. 4.358 têm ≤ 64 px de lado maior, 917 até 256 px, **4 acima
de 1024 px**. 5.444 são `journal-residue` do journal ext4 de uma instalação Linux anterior:
`brave-browser.png`, `git-logo.png`, telas de ajuda do GNOME (`shell-appts.png`, `gnome-logs-3-34.png`).

Da via NTFS vieram 112, e os dois únicos ≥ 300 px são: uma foto de pato-mandarim 960×960 em dois
extents (`partial-or-thumbnail`) e um meme de WhatsApp 720×1080 em dois extents (`fs-metadata`) —
ambos com nome SHA-1 de cache de navegador. **Nenhuma fotografia pessoal.**

**31 artefatos multi-extent no disco inteiro**, 25 `fs-metadata` e 6 rebaixados; 2 acima de 300 px.

## Predições que se confirmaram

- **`mark_current` sempre funcionou** (§6): três volumes vêm `current`, em 2.097.152, 4.502.585.344
  e 74.503.421.952 — exatamente os inícios de partição lidos de `/sys`. A build antiga não tinha
  detector btrfs; não havia bug. **CONFIRMADO.**
- **`confirm_ntfs` faz o seu trabalho**: âncoras NTFS caíram de 29 para **11**.
- **O sombreamento é real**: 13 pares NTFS sobrepostos, o volume genuíno de 300,653 GiB cobrindo
  cinco volumes menores em 309–313 GiB. Benigno nesta corrida — o genuíno ordena primeiro — mas o
  mecanismo está confirmado em dado real.

## Achados novos que a medição produziu

1. **`mark_current` decide por igualdade de início e mais nada.** Uma âncora btrfs em 2.097.152
   reivindicando 74,5 GiB foi marcada `current` porque bate com o início de `sdc1` — que é vfat e
   tem 477 MiB. Nem a família nem o comprimento são conferidos (`residue.rs:110-119`).
2. **O estágio `filesystem` é 64 % da corrida**: 3 h 35 m para 2 h 00 m de varredura. O log mostra a
   parada — itens 3.609 → 3.610 → 3.611 consumindo ~2.000 s **cada**, que é a caminhada de journal
   ext4 sobre âncora falsa. Com 15.157 das 15.173 âncoras falsas, **a Onda 1 passa a ser o trabalho
   de maior valor do backlog, por larga margem.**
3. **`metadata_unconfirmed` mistura três causas** e por isso ainda não é legível: registro sem
   extent nenhum (75.188 candidatos), assinatura ausente no primeiro extent, e a dupla contagem do
   §3(d). Separar é barato e é pré-requisito para ler o 499.788.
4. **2.521.693 registros em uso** — 83 % — são ignorados por política (§3c). Passa a ser a maior
   alavanca de recall inexplorada, e agora tem número.
5. **`journal_deletions` continua 0**, agora sobre 11 volumes NTFS confirmados num disco inteiro.
   O pico de 14.011 registros em 60 s é de **criação**, não de deleção: um instalador ou um cache
   sendo povoado em 2020-11-15. O instante da deleção continua indisponível, e `OPEN-WORK §3.9a`
   continua sendo o teste que ninguém rodou.

## Backlog revisto

| # | Ação | Estado |
| --- | --- | --- |
| 1 | **Onda 1 — confirmar a âncora ext4** | 3 h 35 m de 5 h 35 m em jogo |
| 2 | Separar as três causas de `metadata_unconfirmed` | barato, torna 499.788 legível |
| 3 | Contar o que os 2.521.693 registros em uso conteriam | contador, sem mudança de comportamento |
| 4 | Onda 2 — sombreamento (menor volume que cobre) | confirmado real, benigno aqui |
| 5 | `mark_current` conferir família e comprimento | achado novo |
| ~~6~~ | ~~Onda 3 — geometria inferida~~ | **CANCELADA — refutada pela população** |

---

# Onda 1 — confirmação da âncora ext4

Implementada. Duas camadas, na mesma ordem que a NTFS usa.

**1. Auto-consistência barata** (`ext4.rs`, `from_superblock`). Somadas às cinco checagens que já
existiam, cinco relações que `mke2fs` sempre satisfaz e bytes arbitrários satisfazem só por acaso:

| Campo | Relação exigida |
| --- | --- |
| `s_first_data_block` (20) | exatamente `1` para blocos de 1 KiB, `0` caso contrário |
| `s_blocks_per_group` (32) | ≤ bits de um bloco (o bitmap de blocos é um bloco) |
| `s_inodes_per_group` (40) | ≤ bits de um bloco (o bitmap de inodes é um bloco) |
| `s_inodes_per_group × s_inode_size` | ≤ tamanho do grupo (a tabela de inodes cabe no grupo) |
| `s_inodes_count` (0) | **exatamente** `inodes_per_group × ceil(blocks_count / blocks_per_group)` |
| `s_blocks_count_lo` (4) | ≠ 0 |

**2. Confirmação contra a mídia** (`Ext4::locate`). O análogo exato de `ntfs::locate`: o inode 2 de
qualquer sistema ext é `/`, então lê-se a tabela de inodes do grupo 0 e exige-se que ele seja um
diretório com ao menos os dois links que `.` e `..` dão. Ilegível não é confirmação. Consumida em
`filesystem.rs`, na função que passou a se chamar `confirm_volumes`.

## Predição pré-registrada — escrita antes de rodar

Contra a corrida da Onda 0 (`~/Imagens/argos-onda0`), disco inteiro, mesmos parâmetros:

| Medida | Onda 0 | Predição | Refutado se |
| --- | --- | --- | --- |
| `volumes` | 15.173 | **< 100** | > 1.000 |
| âncoras NTFS | 11 | **exatamente 11** | qualquer outro número |
| âncoras btrfs | 5 (3 `current`) | **exatamente 5, 3 `current`** | qualquer outro número |
| estágio `filesystem` | 3 h 35 m | **< 20 min** | > 1 h |
| artefatos `journal-residue` | 5.444 | **dentro de ±10 %** | < 2.722 |

A última linha é a que importa e é a que pode custar caro. Aqueles 5.444 ícones carregam nomes
coerentes (`brave-browser.png`, `git-logo.png`) casados a PNGs que decodificam, o que só um ext4
real produz — logo parte das 15.157 âncoras é genuína, e a confirmação tem de as manter.
**Se `journal-residue` cair abaixo da metade, a Onda 1 custou recall real e volta atrás.**

## Regressão conhecida que a Onda 1 introduz, e o que a fecha

Um superbloco de *backup* descreve o volume, mas está no início do seu grupo, não a 1024 bytes do
início do volume. `volume_at` calcula `início = âncora − 1024` para qualquer acerto, então para um
backup o início sai errado, `has_root_directory` lê o lugar errado e a âncora é descartada — o que é
correto enquanto o primário existir (o primário já nomeia o mesmo volume), e **é perda quando o
primário foi sobrescrito**. O fechamento é o análogo exato do que `ntfs::locate` faz com a cópia do
setor de boot: `s_block_group_nr` (offset 90) diz de que grupo o backup é, e o início do volume sai
de `âncora − (first_data_block + grupo × blocos_por_grupo) × bytes_por_bloco`.
