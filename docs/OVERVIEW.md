# Argos — ficha técnica

Toda afirmação de comportamento traz `arquivo.rs:linha`; todo número, sua origem.

---

## 1. O que é e o que não é

Recuperação forense de imagens permanentemente apagadas de dispositivos de bloco (HDD, SSD, NVMe) e
de imagens raw. Recupera **apenas JPEG e PNG** — o enum é exaustivo, para que um formato novo não
compile até ter validador, assinatura e mapeamento de relatório
([lib.rs:104](../crates/argos_core/src/lib.rs#L104)). Não escreve na mídia
([source.rs:19](../crates/argos_core/src/source.rs#L19)).  Não recupera dado sobrescrito, bloco
TRIMado, nem JPEG progressivo ou aritmético **quando fragmentado**
([mcu.rs:421](../crates/argos_carve/src/mcu.rs#L421)).

---

## 2. As duas vias de recuperação

**Metadados de sistema de arquivos.** O FS guarda onde o arquivo estava: runlist NTFS, árvore de
extents ext4, cluster inicial FAT, registros APFS/btrfs. Apagar destrói o *ponteiro*, não os bytes;
e um reformat rápido sobrescreve só uma região pequena — os âncoras do FS *anterior* sobrevivem na
superfície e são varridos ([residue.rs](../crates/argos_fs/src/residue.rs)). Devolve **nome, datas e
extents exatos**, e é a única via *verificável*: a mídia confirma a reivindicação duas vezes antes
de ela virar finding ([filesystem.rs:384](../crates/argos_engine/src/pipeline/filesystem.rs#L384)).

**Carving de superfície.** Mortos os metadados, resta a superfície: Argos a varre procurando
assinaturas e conduz cada candidato pela máquina de estados do formato
([carving.rs:14](../crates/argos_engine/src/pipeline/carving.rs#L14)). Não há reivindicação a
verificar — o decodificador *é* a evidência. Contíguo sai inteiro; fragmentado não sai: vira **ponto
de fragmentação** com o offset exato onde o fluxo deixou de ser aquele arquivo, e a §6 tem de
*encontrar* o resto. A via 1 lê um endereço; a via 2 procura por ele.

---

## 3. Arquitetura

Portas e adaptadores. O centro (`argos_engine`) é lógica pura; tudo que toca SO, tela ou disco é
adaptador atrás de uma porta declarada em `argos_core`: `BlockSource`, `ArtifactSink`, `Classifier`,
`ProgressSink`. Onde `std` já dá a abstração, o trait de `std` **é** a porta — parsers e carvers
consomem `impl Read + Seek`, e por isso são testáveis em memória.

```
   argos (CLI + raiz de composição)         argos_ui (Tauri) ──▶ argos_ipc
     │                                          └─ não alcança o motor
     ├─▶ argos_engine ─┬─▶ argos_fs        ─▶ argos_core   ← as PORTAS
     │    (o centro)   ├─▶ argos_carve     ─▶ argos_core     BlockSource · ArtifactSink
     │                 └─▶ argos_classify  ─▶ argos_core     Classifier  · ProgressSink
     ├─▶ argos_device ─▶ argos_core   (HAL por SO; único crate com `unsafe`)
     └─▶ argos_report ─▶ argos_core   (manifesto, hashes, cadeia de custódia)
```

**O que a arquitetura impede — no compilador, não na revisão.** `argos_ui` não alcança
`argos_engine`, `argos_carve` nem `argos_fs`: não pode conter lógica de recuperação. `argos_ipc` não
depende de nada do workspace, o que impede um tipo do motor de vazar para o fio. `argos_engine` não
depende de `argos_report`: resultados saem pela porta `ArtifactSink`, que o binário injeta.
`argos_fs`, `argos_carve` e `argos_classify` não dependem uns dos outros nem de `argos_device` —
encontram-se só no motor ([DEVELOPMENT-PLAN §2.2](DEVELOPMENT-PLAN.md)). E `unsafe` cabe num crate
([lib.rs:4](../crates/argos_device/src/lib.rs#L4)).

---

## 4. Pipeline

Ordem de custo e confiança.

| Estágio | Entrada → Saída | O que faz | Parâmetro-chave | Âncora |
| --- | --- | --- | --- | --- |
| **A. Varredura** | mídia → candidatos + âncoras | Uma thread lê a superfície; um pool casa assinaturas **e** âncoras de FS no mesmo bloco | chunk 8 MiB; passo 512 B | [pipeline.rs:545](../crates/argos_engine/src/pipeline.rs#L545) |
| **B. Sistema de arquivos** | volumes → findings | Recupera apagados de NTFS/ext4/FAT/exFAT/APFS/btrfs, atuais e residuais | teto 4.096 | [filesystem.rs:11](../crates/argos_engine/src/pipeline/filesystem.rs#L11) |
| **C. Validação** | candidatos → findings + fragmentação | Cada hit passa pela máquina de estados do formato; quem quebra vira `Broken` com o offset exato | 512 MiB/imagem | [carving.rs:14](../crates/argos_engine/src/pipeline/carving.rs#L14) |
| **D. Remontagem** | pontos de fragmentação → findings | Procura os extents que faltam, região a região em memória | 2 h; 262.144 hipóteses | [reassembly.rs:41](../crates/argos_engine/src/pipeline/reassembly.rs#L41) |
| **E. Emissão** | findings → artefatos + manifesto | Relê, hasheia, mede pixels, escreve — sequencial | piso 300 px | [output.rs:89](../crates/argos_engine/src/pipeline/output.rs#L89) |
| **F. Anotação** | artefatos → rótulos e prévias | Dedup perceptual e rótulo determinístico, após tudo persistido | Hamming 3 | [annotate.rs:117](../crates/argos_engine/src/annotate.rs#L117) |

**A.** A mídia é lida **uma vez** para os dois detectores; o leitor é uma thread só, porque em disco
rotacional a vazão morre sob seeks.

**C.** O ponto de fragmentação vem do decodificador de entropia, não da gramática de marcadores:
esta lê comprimentos de segmento dos próprios bytes desconhecidos e salta até 65.533, pondo o fim
aparente do arquivo dezenas de KB adiante da emenda real
([reassemble.rs:268](../crates/argos_carve/src/reassemble.rs#L268)).

**E.** O piso de 300 px não descarta: o que fica abaixo segue no manifesto com extents, digest,
dimensões e a razão da omissão — basta para uma nova corrida com piso menor
([config.rs:87](../crates/argos_engine/src/config.rs#L87)).

---

## 5. Técnicas e algoritmos, por nome

Âncora por algoritmo no inventário completo, [ALGORITMOS.md](ALGORITMOS.md).

Aquisição multipasse estilo **ddrescue**; leitura **`O_DIRECT`**. Parse **MBR** e **GPT UEFI** com
**CRC-32** e recurso ao GPT de backup. NTFS: **fixups do Update Sequence Array**, **decodificação de
data runs** (delta de LCN assinado), **change journal `$UsnJrnl:$J`**, **slack de `$I30`**. ext4:
**árvore de extents** e **varredura do journal jbd2**. FAT/exFAT: **dirent `0xE5`** com
**reconstrução por hipótese de contiguidade**. APFS: **Fletcher-64**, **B-tree do object map**,
**diff de checkpoints copy-on-write**. btrfs: **CRC-32C**, **B-tree**, **resolução lógico→físico
pela chunk tree**. Carving: **Two-Way (Crochemore–Perrin)** com pré-filtro SIMD, **máquinas de
estado JPEG (T.81)** e **PNG** (CRC-32 por chunk + **inflate zlib/DEFLATE**), **walker TIFF/IFD**.
Oráculo: **Huffman canônica** com **tabela de lookup direto** de 10 bits, **DPCM do coeficiente
DC**, **run-length do AC**, **de-stuffing `FF 00`**, **decodificação incremental MCU a MCU** com
**retomada** na fronteira. Remontagem: **entropia de Shannon**, **Bifragment Gap Carving**,
**Parallel Unique Path**, **busca em feixe (beam search)** em profundidade com **poda
branch-and-bound**, **fusão de intervalos + busca binária**, **MAD de luminância normalizada** na
costura, **enxerto de cabeçalho em ponto `RSTn`**. Emissão: **SHA-256**, **blockhash perceptual
8×8** com **distância de Hamming**, **segunda diferença de luminância**, **runs de vizinhos do mesmo
tamanho**.

---

## 6. Remontagem de fragmentos

**O espaço de busca.** FSs alocam em clusters, então toda fronteira de fragmento cai num múltiplo de
4 KiB — o menor cluster que qualquer FS suportado usa
([reassemble.rs:41](../crates/argos_carve/src/reassemble.rs#L41)). A grade parte do início do
*volume* da região, não da mídia: um volume desalinhado põe toda fronteira real fora da grade
absoluta ([reassembly.rs:383](../crates/argos_engine/src/pipeline/reassembly.rs#L383)). Cada bloco é
classificado por entropia e stuffing, separando o que pode conter imagem do resto
([classify.rs](../crates/argos_carve/src/classify.rs)) — é isso que torna a busca finita.

**Por que a ótima é intratável.** Ordenar *n* fragmentos é caminho hamiltoniano de peso máximo num
grafo de adjacência. No lugar, **Parallel Unique Path**: cada cabeçalho cresce seu caminho, a cada
passo tomando o fragmento que leva o decodificador mais longe, e e um extent já reivindicado não é
oferecido a outro ([reassemble.rs:732](../crates/argos_carve/src/reassemble.rs#L732)). Antes dela
roda a busca de lacuna bifragmentada — o padrão dominante, dois fragmentos com um vão — do menor vão
para o maior ([reassemble.rs:359](../crates/argos_carve/src/reassemble.rs#L359)).

**Aceitação — duas provas.** Primeiro o decodificador: a montagem tem de decodificar **todos** os
MCUs que o cabeçalho declara e então alcançar `EOI`. Progresso é MCUs, não posição no fluxo
([mcu.rs](../crates/argos_carve/src/mcu.rs)). Não basta: duas fotografias da mesma câmera
compartilham tabelas de Huffman, e uma emenda entre elas decodifica limpo. O que as separa é a
*imagem* na emenda — linhas de uma foto real mudam gradualmente, uma emendada mostra borda dura. A
montagem é recusada se a linha da costura se destacar mais que **3× a diferença mediana entre linhas
do quadro**, e **toda** emenda tem de passar: quatro fragmentos emendam três vezes, e uma junta
errada faz um arquivo que nunca existiu
([reassemble.rs:86](../crates/argos_carve/src/reassemble.rs#L86),
[reassemble.rs:1340](../crates/argos_carve/src/reassemble.rs#L1340)).

**Por que o teto é 3 fragmentos.** O caminhamento guarda até 3 continuações por nível, mas **só até
três fragmentos** ([reassemble.rs:1000](../crates/argos_carve/src/reassemble.rs#L1000)). O limite
não é orçamento, é o oráculo. Contra verdade plantada, ramificar em três recupera 87% sem nada
fabricado, contra 25% antes. **Em quatro não se sustenta: a suíte produziu uma montagem do
comprimento certo, que decodificou ponta a ponta, cujas três costuras todas passaram, e que não eram
os bytes plantados.** Nenhum limiar de costura separa esse caso sem recusar junto um terço das
recuperações verdadeiras — 2,5 e 2,0 o deixam passar, 1,6 custa esse terço. O limite honesto é a
profundidade ([recovery_rate.rs:290-303](../crates/argos_carve/tests/recovery_rate.rs#L290-L303)).

---

## 7. Garantias

| Garantia | Âncora | Imposta por |
| --- | --- | --- |
| **Somente-leitura.** Dispositivos abrem `O_RDONLY`; a porta não tem escrita, discard nem passthrough | [source.rs:19](../crates/argos_core/src/source.rs#L19), [linux.rs:155](../crates/argos_device/src/device/linux.rs#L155) | **compilador** |
| **Escada de confiança.** Seis níveis, fixados pela evidência que produziu o artefato; nunca elevados | [lib.rs:167](../crates/argos_core/src/lib.rs#L167) | compilador + convenção |
| **Confirmação dupla antes de emitir por metadados.** Assinatura no primeiro extent **e** extents montados passando a máquina de estados do carving; quem falha vira parcial, não é descartado | [filesystem.rs:384](../crates/argos_engine/src/pipeline/filesystem.rs#L384) | **teste** |
| **Proveniência por artefato.** Extents absolutos, estágio, nível e SHA-256 da recuperação; concatená-los dá os bytes entregues | [artifact.rs:53](../crates/argos_core/src/artifact.rs#L53) | teste |
| **Nunca fabricar valor que falhou validação.** Timestamp ausente é `None`; região ilegível é registrada, nunca zerada; finding que não relê é contado, não inventado | [lib.rs:40](../crates/argos_core/src/lib.rs#L40), [output.rs:190](../crates/argos_engine/src/pipeline/output.rs#L190) | teste + convenção |
| **Triagem nunca é veredito.** Roda após tudo persistido; sem caminho de volta a extents ou confiança | [annotate.rs](../crates/argos_engine/src/annotate.rs) | arquitetura |

---

## 8. Por que Rust

Parsers de estrutura de disco corrompida são superfície de ataque: todo comprimento, offset e
contagem lido da mídia é entrada hostil, e o modo de falha clássico é aritmética que estoura seguida
de indexação. Aqui **nenhum acesso a buffer é indexação direta**: todo campo passa por acessores que
devolvem `Option` e fazem `checked_add` antes do slice, então um campo fora do buffer falha o parse
do objeto em vez de derrubar o processo ([lib.rs:232](../crates/argos_fs/src/lib.rs#L232)). Toda
caminhada auto-referenciada tem limite nomeado, o que faz ciclo forjado terminar: extents ext4 em 5
níveis ([ext4.rs:36](../crates/argos_fs/src/ext4.rs#L36)), B-tree APFS em 8, runs NTFS em 4.096; e
um `read` nunca passa de 8 MiB. `unsafe` cabe em **um** crate, 18 blocos em syscall/ioctl da HAL;
nos outros nove é zero, e cada bloco carrega comentário de segurança exigido por lint. Fecham o
argumento **17 alvos de fuzzing** rodados 120 s cada a todo push, e Miri sobre `argos_device` nos
três SOs ([ci.yml](../.github/workflows/ci.yml)).

---

## 9. Como se valida

**Fixtures com bytes plantados.** Os construtores geram volumes e imagens estruturalmente válidos
com um arquivo apagado conhecido, mais as variantes que todo parser tem de sobreviver: truncamento,
comprimento estourado, referência cruzada em ciclo, zero-fill
([fixture.rs](../crates/argos_fs/src/fixture.rs)). A asserção decisiva não é contagem: a suíte de
remontagem compara **os bytes reivindicados contra os bytes plantados**, e uma resposta que
decodifica mas difere do plantado conta como *fabricada* e reprova a suíte — ainda que a taxa tenha
subido ([recovery_rate.rs:167](../crates/argos_carve/tests/recovery_rate.rs#L167)). **340 testes, 0
falhas, 2 ignorados** (`cargo test --workspace --release`, 2026-08-26).

**"O teste passa" ≠ "recupera em mídia real".** Toda fixture aqui é plantada por este projeto, e uma
busca ajustada contra as próprias fixtures é o modo de falha que uma taxa não revela. O arranjo para
corpora públicos — DFRWS 2006/2007, NIST CFReDS FC-01..FC-05 — lê os dados de `ARGOS_CORPUS_DIR` e
decide recuperação **só por digest**
([corpus_recall.rs](../crates/argos_engine/tests/corpus_recall.rs)). **Nunca foi rodado.**

---

## 10. Números

Todos de campo; viés na mesma linha. Negativos em **negrito**.

| Medição | Valor | Origem e ressalva |
| --- | --- | --- |
| Corrida analisada | `/dev/sdc`, 1 TB, 12 workers, 5 h 31 m | Disco mecânico de 10 anos, NTFS→Linux. `OPEN-WORK §1`; **as cinco linhas seguintes são dela** |
| Manifesto / escritos | 348.361 / 47.658 | §1.1 |
| **Omitidos sob o piso de 300 px** | **300.703** | §1.1. Nada perdido — extents e digest retidos —, mas 86% do manifesto não chega ao diretório |

| **Assinaturas que falharam validação** | **388.301** | §1.1. Mede quanto ruído a superfície oferece |
| Pontos de fragmentação | 50.355 (42.484 PNG) | §1.3 |
| **Remontagem: tentados / recuperados** | **254 / 3, teto de 2 h atingido** | §1.5. 0,5% da fila: o orçamento foi testado, a busca não |
| **Custo de uma hipótese** | 2,5–5,7 µs em ruído vs **573–580 µs entre fotografias** | `defects/07`. O estágio é mais lento exatamente onde o alvo está |
| Recuperações por metadados | 91 (1 TB); 111 (corrida dirigida NTFS) | `OPEN-WORK §1.1`; `RELATORIO-RUNLISTS-ORFAS §2` |
| Fragmentos por runlist recuperada | 1→22, 2→30, 3→1 | `RUNLISTS-ORFAS §1.4`. **Viés: só sobre runlists que recuperaram** — as que falharam não aparecem, a distribuição é truncada |
| Fragmentado: metadados vs carving | 57% das runlists vs **101 de 100.535 (0,1%)** do carving | Idem. Metadados é a única via que recupera fragmentado a taxa |
| Taxa por padrão de fragmentação | 87% (2 e 3 fragmentos, ordem invertida, foto competindo); **25% em 4**; **0 fabricados nos 6** | `cargo test -p argos_carve --test recovery_rate --release`, 2026-08-26. **Viés: fixtures sintéticas próprias, 8 amostras por padrão** |

---

## 11. Limitações

**De implementação.** O oráculo PNG não tem retomada — revalida o caminho inteiro por hipótese, e
90% da fila era PNG (`OPEN-WORK §3.1`). Os pontos de reentrada `RSTn` são computados e **não
ligados** ao grafo (`§3.3`). A varredura ext4 aceita um superbloco sem confirmar que há volume
atrás: **15.157 volumes falsos contra 29 NTFS** (`§3.6`). HPA/DCO não são endereçados **nem
declarados** — o que fica atrás de área protegida sai de toda varredura e o relatório não diz
(`§3.9d`).

**Limite teórico.** A remontagem exata é intratável e a caminhada gulosa com poda é a resposta certa
— mas o limite que morde é mais próximo que o computacional: **em quatro fragmentos o custo de junta
para de distinguir uma montagem real de uma plausível** (§6). Um buraco num JPEG baseline sem `DRI`
é permanente dali em diante: predição DC é diferencial e Huffman não ressincroniza; só `RSTn` cria
reentrada, e só se a câmera os escreveu (`OPEN-WORK §5`).

**Limite físico.** Em disco magnético usado, a probabilidade medida de recuperar 1.024 bits
sobrescritos é 1,4×10⁻²⁵⁸ (Wright et al. 2008, Tabela 1); um JPEG de 1 MB tem 8×10⁶ bits. Não é
limitação de ferramenta, é o meio. Miniatura de cache prova que a foto existiu, mas **não reconstrói
a original**.

---

## 12. Referências

**Especificações e documentação de kernel** — não são papers: ITU-T T.81 (JPEG); ISO 15948 / RFC
2083 (PNG); RFC 1950/1951 (zlib/DEFLATE); Microsoft FAT32 1.03 e exFAT; *Apple File System
Reference*; ext4/jbd2 e btrfs on-disk format do kernel Linux; UEFI (GPT); NTFS conforme
ntfs-3g/libntfs — **a única família sem especificação normativa pública**.

**Trabalhos acadêmicos.** O interessante em cada um é a divergência.

1. **Garfinkel, S.** "Carving contiguous and fragmented files with fast object validation", *Digital
   Investigation* 4S (2007) S2–S12.
   *Tirado:* bifragment gap carving, e as estatísticas de fragmentação real (6% dos arquivos sobre 2.143.553; pior caso, 43% dos JPEGs de um disco).
   *Divergência:* o artigo pareia cabeçalho **e rodapé**; Argos não enumera rodapés — o segundo
   fragmento corre até o fim da região e é aparado pelo decodificador, senão o resultado dependeria de
   qual `FF D9` falso viesse primeiro
   ([reassemble.rs:349](../crates/argos_carve/src/reassemble.rs#L349)).

2. **Pal, A., Sencar, H.T. & Memon, N.** (DFRWS 2008); **Pal, A. & Memon, N.** (*IEEE Signal
   Processing Magazine*, 2009). `[títulos exatos a confirmar]`
   *Tirado:* a remontagem como caminho em grafo, sua intratabilidade, e o **Parallel Unique Path**.
   *Divergência:* PUP puro é guloso e nunca reconsidera; aqui ramifica em até 3 continuações por nível
   **e só até 3 fragmentos** — corte vindo de medição própria (§6).

3. **Uzun, E. & Sencar, H.T.** "Carving Orphaned JPEG File Fragments", *IEEE TIFS* 10(8):1549–1563,
   2015.
   *Tirado:* recuperar pixels de um fragmento sem cabeçalho, estimando os parâmetros de codificação de
   um corpus de câmeras.
   *Divergência:* Argos não estima — se um arquivo do mesmo lote sobreviveu, os parâmetros são
   *conhecidos* e o cabeçalho dele é emprestado. Fica **fora do pipeline**, no nível `Grafted`, porque
   aqueles bytes naquela ordem nunca estiveram na mídia
   ([graft.rs](../crates/argos_engine/src/graft.rs)).

4. **Wright, C., Kleiman, D. & Sundhar R.S., S.** "Overwriting Hard Drive Data: The Great Wiping
   Controversy", ICISS 2008, LNCS 5352, pp. 243–257.
   *Tirado:* o limite físico da §11 — 1,4×10⁻²⁵⁸ para 1.024 bits num drive usado, por microscopia de
   força magnética sobre 76.800 pontos.
   *Divergência:* nenhuma; é o único limite aqui que versão futura alguma move. Posição normativa:
   NIST SP 800-88 Rev. 1.

---

## 13. Lacunas — confirmar antes de apresentar

1. **Títulos exatos de Pal/Sencar/Memon (2008) e Pal/Memon (2009).** O código dá autor, veículo e
   ano; o título, nenhum documento do repositório dá.
2. **Fitzgerald et al.**, sem autor completo, título ou ano em
   [classify.rs:7](../crates/argos_carve/src/classify.rs#L7) — `[origem a confirmar]`.
3. **Quick & Tassone, "Forensic Analysis of Windows Thumbcache files"** — sem ano confirmado
   (`audit/RELATORIO-RECUPERABILIDADE.md §A5`).
4. **Nenhum recall contra corpus público existe.** A "qual o recall em mídia real?" a resposta
   honesta é *não medido*.
5. **138 MB/s de varredura** (`OPEN-WORK §1.2`): uma máquina, um disco, não repetida — e a corrida
   de 1 TB da §10 é uma só, logo todo número dela é *n=1*.
