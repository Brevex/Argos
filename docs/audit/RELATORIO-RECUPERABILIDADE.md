# Relatório — recuperabilidade do lote de fotografias excluído no Windows

Produzido contra [PROMPT-RECUPERABILIDADE.md](PROMPT-RECUPERABILIDADE.md). Toda proposição carrega
etiqueta: `[MEDIDO]` deste repositório, `[CÓDIGO]` com `caminho:linha`, `[PUBLICADO]` com achado
específico, `[INFERIDO]` com premissas nomeadas, `[DESCONHECIDO]` com a medição que a encerraria.

**A condição de independência da §0 do prompt não foi satisfeita.** Ver Fase G.

---

## Sumário executivo

1. **A afirmação "estas fotografias não podem mais ser recuperadas" não pode ser feita hoje.** Não
   porque a evidência aponte para o contrário, mas porque o critério de irrecuperabilidade (Fase E)
   falha no segundo dos três requisitos: técnicas aplicáveis permanecem não tentadas. `[INFERIDO]`
2. **A afirmação inversa também não pode ser feita.** Nenhuma hipótese de sobrevivência foi
   confirmada por medição discriminante. `[INFERIDO]`
3. **A parte física da pergunta tem resposta definitiva e ela é dura.** Em disco magnético, byte
   sobrescrito não volta: num drive usado, a probabilidade de recuperar 1024 bits sobrescritos é
   1,4×10⁻²⁵⁸. Um JPEG tem ~10⁷ bits. `[PUBLICADO]` A pergunta se reduz a *se* houve sobrescrita, o
   que é mensurável.
4. **A busca por remontagem nunca rodou.** 254 de 50.355 pontos atendidos = 0,5%, teto de 2 h
   atingido. `[MEDIDO]` E ela é duas ordens de grandeza mais lenta exatamente nas regiões densas em
   fotografias, que é onde o alvo está. `[MEDIDO]`
5. **Três hipóteses testáveis sem escrever uma linha de código** — o manifesto existente pode já
   conter o lote (300.703 artefatos sob o piso de 300 px; 1.512 listas de runs órfãs). `[MEDIDO]`
6. **A maior lacuna publicada não é em carving, é de premissa**: Argos não recupera nenhum fragmento
   sem cabeçalho. `[CÓDIGO]` O estado da arte publicado para JPEG recupera **24% mais dados de
   imagem** que os carvers existentes fazendo exatamente isso. `[PUBLICADO]`
7. **Uma exclusão dentro do Windows deixa seus rastros mais fortes em artefatos que Argos não lê**:
   thumbcache, `$LogFile`, `$Recycle.Bin`, bancos do OneDrive. Zero parsers. `[CÓDIGO]`
8. **Nada disso foi medido contra verdade-fundamental.** P9 pendente; DFRWS 2006/2007 e NIST CFReDS
   existem, são públicos e são baixáveis. `[PUBLICADO]`

---

## Fase A — estado da arte publicado

### A1. Carving e remontagem

`[PUBLICADO]` Garfinkel, S. "Carving contiguous and fragmented files with fast object validation",
*Digital Investigation* 4S (2007) S2–S12 (DFRWS 2007). Corpus de 300+ discos de segunda mão,
2.143.553 arquivos com dados, 892 GB. Achados com número:

- **6% dos arquivos (125.659) estavam fragmentados.** Metade dos discos não tinha *nenhum* arquivo
  fragmentado; 30 discos tinham mais de 10%.
- **Arquivos de interesse forense (AVI, DOC, JPEG, PST) fragmentam significativamente mais** que os
  de pouco interesse (BMP, HLP, INF, INI).
- **Fragmentação sobe com uso prolongado, disco cheio e muitos ciclos de criar/apagar** — as três
  condições sob as quais o SO é obrigado a fragmentar.
- Pior caso medido: **43% dos 2.517 JPEGs de um disco de 14 GB estavam fragmentados**; outro a 34%,
  outro a 33%.
- Distribuição de lacunas em JPEG bifragmentado (Tabela 5): moda em **2 bytes** (99 arquivos), depois
  **4096 bytes = 8 setores** (88 arquivos).
- Bifragment Gap Carving: para cabeçalho e rodapé conhecidos, varrer tamanhos de lacuna validando
  cada hipótese com o decodificador real. Custo O(n⁴) para achar todos os objetos bifragmentados.

`[INFERIDO]` (de A1 + §2 do caso) O perfil do disco do usuário — ~10 anos de uso, múltiplas
reinstalações, ciclos de criar/apagar — é o perfil de *alta* fragmentação de Garfinkel, não o de
metade dos discos que não tinham nenhuma. Fotografias estão na categoria que fragmenta mais.

### A2. Fragmentos órfãos e arquivos sem cabeçalho — **a subseção decisiva**

`[PUBLICADO]` Uzun, E. & Sencar, H.T. "Carving Orphaned JPEG File Fragments", *IEEE Transactions on
Information Forensics and Security*, vol. 10, n. 8, pp. 1549–1563, 2015. Recupera fragmentos de JPEG
**quando o cabeçalho não existe mais** — o paradigma de carving vigente exige conhecer as
configurações de compressão e codificação para ter sucesso, e este trabalho remove essa exigência.

`[PUBLICADO]` Uzun, E. & Sencar, H.T. "JpgScraper: An Advanced Carver for JPEG Files", *IEEE TIFS*,
vol. 15, pp. 1846–1857, 2020. Números:

- **24% mais dados de imagem recuperados** que ferramentas de carving existentes, em cartões SD
  usados.
- Identifica cabeçalhos JPEG parciais com **zero falsa rejeição e 0,1% de falso alarme**.
- Discrimina dados JPEG entre **993 tipos de dados com 97,7% de acurácia**.
- Base construída sobre **mais de 7 milhões de imagens**, cobrindo parâmetros de codificação de
  **3.269 modelos de câmera**.
- Implementações públicas: `github.com/euzun/jpgscraper`, `github.com/euzun/jpeg-carver-csharp`.

`[INFERIDO]` (de A2) A dificuldade central da técnica é *não saber* os parâmetros de codificação —
daí a base de 3.269 câmeras. Quem possui arquivos sobreviventes do mesmo lote e da mesma câmera
conhece exatamente as tabelas de Huffman, as tabelas de quantização e os fatores de subamostragem.
A parte cara do método publicado é gratuita neste caso.

### A3. Classificação de fragmentos

`[PUBLICADO]` Mittal, G. *et al.* "FiFTy: Large-Scale File Fragment Type Identification Using
Convolutional Neural Networks", *IEEE TIFS* vol. 16, 2021. **77,5% de acurácia média sobre 75 tipos
de arquivo a ~38 s/GB**, contra o estado da arte anterior (Sceadan) a **69% e 9 min/GB** — melhor e
mais de uma ordem de grandeza mais rápido.

`[INFERIDO]` FiFTy resolve um problema de 75 classes; Argos precisa de uma decisão binária
("este bloco pode conter dados de imagem?"). Um detector especializado de fluxo entrópico JPEG não é
comparável a um classificador geral por acurácia agregada, e a ausência de FiFTy em Argos **não** é
por si uma lacuna de recall. `[DESCONHECIDO]` A acurácia binária do detector de
`crates/argos_carve/src/classify.rs` nunca foi medida contra rótulos conhecidos. Encerraria a
questão: rodá-lo sobre o conjunto FiFTy restrito a JPEG-vs-resto.

### A4. Artefatos NTFS além do `$MFT`

`[PUBLICADO]` "NTFS Data Tracker: Tracking file data history based on $LogFile", *Forensic Science
International: Digital Investigation* (Elsevier), 2021. O `$LogFile` registra operações redo e undo
das atualizações de metadados NTFS em termos de LSN, incluindo criação e **exclusão** de arquivo e
modificação de entrada MFT; cada registro guarda o dado *depois* (redo) e *antes* (undo) da
operação. Pesquisa anterior citada nele já extraía **as data runs (localização dos dados) a partir da
entrada MFT armazenada no redo** dos registros correspondentes.

`[INFERIDO]` (de A4) O `$LogFile` pode conter a runlist `$DATA` de um arquivo excluído mesmo quando o
registro `$MFT` correspondente já foi reutilizado. É a única fonte de extents que sobrevive à
reutilização do registro. `[DESCONHECIDO]` Por quanto tempo, para este disco — o `$LogFile` é
circular e de tamanho fixo, e houve reinstalações. Encerraria: localizar `$LogFile` residuais na
superfície e datar seus registros.

### A5. Artefatos do Windows que guardam pixels

`[PUBLICADO]` Quick, D. & Tassone, C. *et al.*, "Forensic Analysis of Windows Thumbcache files".
O cache fica em `%USERPROFILE%\AppData\Local\Microsoft\Windows\Explorer`, um arquivo por tamanho:
`thumbcache_32.db`, `_96`, `_256`, `_1024`, mais `thumbcache_idx.db`. **Uma miniatura pode
permanecer no cache depois que a imagem original foi excluída**, porque o cache não é atualizado
imediatamente. Em vários processos judiciais a miniatura *foi* a evidência apresentada. Análise
adicional pode ligar a miniatura ao arquivo de origem, incluindo **caminho completo e nome original**.

`[INFERIDO]` (de A5 + `docs/defects/02`) O `defects/02` deste repositório chegou independentemente à
mesma conclusão por análise de vizinhança — 51 de 60 artefatos num raio de ±4 MiB com exatamente
256×192 — e a registrou corretamente: um cache sobrevive às fotografias que descreve porque arquivos
grandes são sobrescritos primeiro. A conclusão do `defects/02` de que **a original não se reconstrói
a partir da miniatura** está correta e não é contestada aqui.

### A6. OneDrive

`[PUBLICADO]` Logs ODL em `%LOCALAPPDATA%\Microsoft\OneDrive\logs` (`SyncEngine.odl`, comprimidos
como `.odlgz`) registram operações por arquivo processado e permitem reconstruir uploads, downloads,
renomeações e **exclusões**. `SyncEngineDatabase.db` e `UserCid.dat` em
`%LOCALAPPDATA%\Microsoft\OneDrive\settings` expõem a estrutura dos dados sincronizados.

`[INFERIDO]` (de A6) Isso devolve **a lista do que se perdeu** — nomes, caminhos, momentos — não
pixels. Para um lote cujos nomes o usuário não tem, essa lista é o que converte uma busca cega numa
busca por assinatura conhecida (ver Fase F).

### A7. Física do overwrite em mídia magnética — **a resposta dura**

`[PUBLICADO]` Wright, C., Kleiman, D., Sundhar R.S., S. "Overwriting Hard Drive Data: The Great
Wiping Controversy", ICISS 2008, LNCS 5352, pp. 243–257. Estudo empírico com microscopia de força
magnética, 76.800 pontos de dados. **Tabela 1** — probabilidade de recuperação após uma sobrescrita:

| Bits | Drive "pristine" (melhor caso) | Drive usado (caso ideal) |
| --- | --- | --- |
| 1 bit | 0,92 | **0,56** |
| 8 bits (1 caractere) | 0,513 | 0,0097 |
| 32 bits | 0,0694 | 8,75×10⁻⁹ |
| 512 bits | 2,88×10⁻¹⁹ | 1,2×10⁻¹²⁹ |
| 1024 bits | 8,29×10⁻³⁸ | **1,4×10⁻²⁵⁸** |

Conclusão textual do artigo: dados corretamente sobrescritos não podem ser razoavelmente recuperados
"nem com o uso de MFM ou outros métodos conhecidos"; e é necessário que o dado tenha sido escrito e
apagado num disco **novo e não usado** para haver qualquer esperança de recuperação em nível de bit,
"o que não reflete situações reais" — a interação de desfragmentação, cópias e uso geral anula
qualquer chance.

`[PUBLICADO]` NIST SP 800-88 Rev. 1, *Guidelines for Media Sanitization*: para dispositivos com mídia
magnética, **uma única passagem de sobrescrita** com padrão fixo tipicamente impede a recuperação
"mesmo que técnicas laboratoriais de estado da arte sejam aplicadas".

`[INFERIDO]` (de A7) Uma fotografia de 1 MB tem 8×10⁶ bits. Extrapolando a tabela, a probabilidade de
recuperá-la de região sobrescrita num drive usado é indistinguível de zero por qualquer critério.
**Onde houve sobrescrita, a perda é definitiva e isso é certeza científica, não estimativa.** O que
a literatura *não* permite afirmar: que a região do lote *foi* sobrescrita. Isso é uma questão de
medição da mídia, não de física.

### A8. Corpora com verdade-fundamental

`[PUBLICADO]` **DFRWS 2006 Carving Challenge**: arquivo de 49.999.872 bytes com blocos de texto,
Office, **JPEG** e ZIP, sem metadados de sistema de arquivos; alguns contíguos, outros em dois ou
três fragmentos. **DFRWS 2007 Challenge**: mais tipos e cenários de fragmentação mais complexos;
disponível em `github.com/dfrws/dfrws2007-challenge`.

`[PUBLICADO]` **NIST CFReDS File Carving** (`cfreds.nist.gov/FileCarving`): casos de teste cobrindo
contíguo (**FC-01, FC-02, FC-03**), fragmentado (**FC-04, FC-05**), com **FC-05 sendo o cenário de
cluster não alinhado**, além de cenários com padding e deslocamento de bytes. Imagens de disco de
exemplo fornecidas por cenário.

---

## Fase B — o que Argos realmente faz

`docs/CAPABILITIES.md`, `docs/OPEN-WORK.md` e `docs/defects/` foram conferidos contra o código. **Não
foi encontrada nenhuma divergência entre o declarado e o implementado.** O repositório documenta as
próprias lacunas com precisão incomum, e vários achados desta auditoria são leituras da documentação
dele confirmadas no código, não descobertas.

Confirmações relevantes:

- `[CÓDIGO]` `crates/argos_engine/src/config.rs:87` — `DEFAULT_MIN_LONG_SIDE = 300`. O doc na
  `:263-265` declara: todo artefato abaixo do piso "is still recorded with its extents, digest and
  dimensions, so the manifest stays a complete account of the medium and **the extents locate the
  bytes exactly for a rerun with a lower floor**". `min_long_side: 0` escreve tudo.
- `[CÓDIGO]` `crates/argos_engine/src/config.rs:72` — `DEFAULT_REASSEMBLY_BUDGET = 2 h`.
- `[CÓDIGO]` `crates/argos_report/src/lib.rs:436,752` — `omitted_because` preserva a razão da omissão
  por artefato.
- `[CÓDIGO]` **Toda recuperação parte de um cabeçalho.** `bifragment()`
  (`crates/argos_carve/src/reassemble.rs:360`) recebe `Broken { header, break_at }`;
  `prefix_candidates(header, break_at, limits)` (`:697`) enumera fins do primeiro fragmento a partir
  do cabeçalho; `parallel_unique_path()` (`:713`) "grows the best extent path **for each broken
  candidate**". Não existe caminho que produza dados a partir de um fragmento sem cabeçalho.
- `[CÓDIGO]` `crates/argos_carve/src/reassemble.rs:1445-1447` — `restart_points()` existe e seu
  próprio doc declara: "wiring them in as first-class nodes **is not implemented**".
- `[CÓDIGO]` `crates/argos_fs/src/part.rs:31` — `SECTOR: u64 = 512`, com justificativa documentada.
- `[CÓDIGO]` Busca global por `LogFile`/`LSN` em `crates/`: **zero ocorrências**.
  (`grep -rni "logfile" --include=*.rs crates/ | wc -l` → 0)

Sobre o alerta de fixtures do prompt: `[MEDIDO]` `OPEN-WORK §3.9a` já admite que o nome do stream
`$UsnJrnl` e o layout `USN_RECORD_V2` são não verificados contra mídia real. `[DESCONHECIDO]` Se há
outros casos da mesma classe. Encerraria: cruzar cada `fixture.rs` com as constantes que o leitor
correspondente consome, e marcar os pares que compartilham a mesma constante.

**Granularidade da busca de lacuna.** `[CÓDIGO]` `bifragment()` avança `second_start` por
`step = limits.block_bytes` (`:378`, `:430`) a partir de `limits.ceil(first_end + 1)` (`:406`), e
`prefix_candidates` recua por `block_bytes` (`:702`). Ambas as pontas são presas à grade de alocação
do volume. `[INFERIDO]` A moda de 4096 bytes / 8 setores da Tabela 5 de Garfinkel é coberta; a moda
de **2 bytes** não é representável nessa grade. `[DESCONHECIDO]` Se uma lacuna de 2 bytes é um
fenômeno físico de alocação NTFS ou um artefato do método de medição de Garfinkel. Encerraria: ler a
definição de gap da §3.4 do artigo e conferir se ela mede fim-de-dados ou fim-de-cluster. **Não trato
isso como lacuna confirmada.**

---

## Fase C — matriz de lacunas

Ordenada por (ganho × probabilidade de aplicar-se) ÷ custo.

| # | Técnica | Fonte | Status em Argos | Ganho neste caso | Custo |
| --- | --- | --- | --- | --- | --- |
| 1 | Consultar o manifesto já produzido | — | Capacidade existe (`report --all`, C34) | **Máximo** — pode encerrar H3/H4 hoje | Minutos, zero código |
| 2 | Completar a busca de remontagem | Garfinkel 2007; Pal/Sencar/Memon | Implementada, **0,5% executada** | **Máximo** — é a técnica certa, não rodada | ~15 h de máquina |
| 3 | Carving de fragmento órfão sem cabeçalho | Uzun & Sencar 2015; JpgScraper 2020 (**+24% de dados**) | **Ausente**; `restart_points()` existe e não é usado | **Alto** — e os sobreviventes dão os parâmetros de graça | Alto |
| 4 | Thumbcache como fonte de pixels | Quick & Tassone | **Sem parser**; só heurística de vizinhança (`cache_run.rs`) | **Alto** — devolve a imagem em 256/1024 px e o nome original | Médio |
| 5 | `$LogFile` | NTFS Data Tracker, FSI:DI 2021 | **Ausente por completo** | **Alto** — única fonte de runlist que sobrevive à reutilização do registro MFT | Médio-alto |
| 6 | Validação contra verdade-fundamental | DFRWS 2006/2007; NIST CFReDS | **P9 pendente** | **Alto, indireto** — sem isso nenhum recall é afirmável | Médio |
| 7 | OneDrive ODL / SyncEngineDatabase | A6 | Ausente | **Médio** — devolve a lista, não os pixels | Médio |
| 8 | `$Recycle.Bin` `$I`/`$R` | prática consolidada | Ausente | **Médio** — exclusão foi dentro do Windows | Baixo |
| 9 | Geometria inferida de registros órfãos | — | `OPEN-WORK §3.8`, não implementado | **Médio-alto** — 1.512 runlists descartadas | Médio |
| 10 | Confirmação da âncora ext4 | — | `OPEN-WORK §3.6` | **Médio** — 15.157 falsos poluem o estágio | Baixo |
| 11 | TIFF | — | `OPEN-WORK §3.9e` | **Médio** — 3 scanners de mesa no corpus | Médio |
| 12 | Classificação de fragmento por CNN | FiFTy, TIFS 2021 | Ausente por decisão (`A-INFERENCE-PURE-RUST`) | **Baixo** — problema binário, não de 75 classes | Alto |
| 13 | Recuperação de dado sobrescrito por MFM | Gutmann 1996 | Ausente | **Nulo** — refutado empiricamente (A7) | — |

**A única mudança, se só uma fosse possível:** ligar `restart_points()` como nós de primeira classe
no grafo de remontagem (`OPEN-WORK §3.3`). É a que fecha a maior lacuna publicada de recall
(fragmento órfão), já está metade construída, e é a única cuja dificuldade central — conhecer os
parâmetros de codificação — este caso resolve de graça pelos sobreviventes.

**Mas a ação de maior valor não é uma mudança.** É a linha 1 da tabela, e não exige código nenhum.

---

## Fase D — por que apenas menos de 10 de um lote homogêneo

Ranking por evidência, não por plausibilidade.

**H2 — presentes e fragmentados, a busca nunca chegou neles. Melhor sustentada.**
`[MEDIDO]` `OPEN-WORK §1`: 254 de 50.355 pontos atendidos (0,5%), 3 recuperados, teto de 2 h
atingido exatamente. `[MEDIDO]` `defects/07`: 6,21 s por passo sobre fila de 46.345 = 80 h
pré-paralelização, sob 15 h depois. `[MEDIDO]` `defects/07`: uma hipótese custa 573–580 µs em região
de fotografias contra 2,5–5,7 µs em ruído — **o estágio é mais lento exatamente onde o alvo está**, e
o `defects/07` já observa que as fotografias procuradas foram gravadas em lotes (113 quadros de uma
câmera dentro de 0,3 GiB). `[PUBLICADO]` (A1) O perfil do disco é o de alta fragmentação de
Garfinkel. **Discriminante:** `reassemble --from` sobre o manifesto existente, sem orçamento,
`--range` na vizinhança dos sobreviventes.

**H3 — recuperados e não escritos. Segunda melhor, e a mais barata de testar.**
`[MEDIDO]` 300.703 artefatos omitidos sob o piso de 300 px, contra 47.658 escritos.
`[CÓDIGO]` `config.rs:263-265` garante que cada um retém extents, digest e dimensões.
**Discriminante:** consultar o manifesto por dimensão, câmera e data, ignorando o diretório de saída.
Custo: minutos.

**H4 — os runlists existem e foram descartados. Terceira.**
`[MEDIDO]` 1.512 regiões de registros `FILE` órfãos descartadas por falta de volume; 91 recuperações
por metadados no total. `[CÓDIGO]` `OPEN-WORK §3.8` descreve a inferência de geometria que resolveria
isso e declara que não está implementada. **Discriminante:** o campo `lost_files` do manifesto — os
registros trazem nome, tamanho, `first_cluster` e contagem de clusters mesmo sem extents.

**H6 — a evidência sobrevivente é o cache, não o arquivo. Estabelecida para *parte* do resultado.**
`[MEDIDO]` `defects/02`: artefato de 256×192 com 51 de 60 vizinhos em ±4 MiB do mesmo tamanho exato.
`[PUBLICADO]` (A5) confirma o mecanismo e acrescenta que o cache pode reter o **nome e caminho
original**. **Discriminante:** `same_size_neighbours` e as dimensões dos menos de 10 sobreviventes.
Se todos forem ≤256 px com vizinhos de tamanho idêntico, o que sobreviveu foi o cache — e as
originais são outra questão, não a mesma.

**H1 — a região foi sobrescrita.** `[DESCONHECIDO]`. É a hipótese que a Fase E precisa resolver e
nenhuma medição a favor ou contra existe hoje. `[MEDIDO]` O único dado tangencial: 15.186 volumes
localizados, **nenhum atual**, com 15.157 deles falsos positivos ext4 (`§3.6`) — o que significa que
o mapa de volumes residuais não é confiável o bastante para dizer o que ocupa a região do lote.
**Discriminante:** o que ocupa hoje os offsets vizinhos aos sobreviventes, e a origem desse conteúdo.

**H5 — não são JPEG/PNG baseline.** `[DESCONHECIDO]`, e barato de resolver. `[CÓDIGO]` Progressivo e
aritmético retornam `ScanStop::Unsupported` (`mcu.rs:421-422`); TIFF não é carveado. **Discriminante:** o
formato exato dos sobreviventes — `SOF0`/`SOF1` versus `SOF2`.

**H7 — os sobreviventes têm propriedade que os demais não têm.** `[DESCONHECIDO]`.
**Discriminante:** comparar tamanho, contagem de extents e offset dos sobreviventes contra a
distribuição do manifesto. Se todos tiverem exatamente 1 extent, H2 sobe; se forem todos pequenos,
H6 sobe.

**H2 e H3 explicam os dados igualmente bem e não são mutuamente exclusivas.** Ambas preveem que o
lote está no disco e ausente do diretório de saída, por motivos diferentes e em estágios diferentes.

---

## Fase E — o teste de irrecuperabilidade

Critério fixado antes de olhar o resultado. Três requisitos; a falha de qualquer um invalida a
afirmação "estas fotografias não podem mais ser recuperadas".

**1. Contabilidade de cobertura — PARCIALMENTE SATISFEITO.**
`[MEDIDO]` A corrida cobriu byte 0 até a capacidade, registrou `coverage` e 98 regiões ilegíveis
custando 0 achados. `[CÓDIGO]` Mas HPA/DCO não são endereçados **nem declarados**
(`OPEN-WORK §3.9d`): setores atrás de uma área protegida ficam fora de toda varredura e o relatório
não diz que ficaram. `[INFERIDO]` Um argumento de cobertura que não declara o que não pôde ver não
fecha. O buraco é provavelmente pequeno, mas "provavelmente pequeno" é hedge portante e a §1 o
proíbe como sustentáculo.

**2. Exaustão de técnica — NÃO SATISFEITO, e não por pouco.**
`[MEDIDO]` A técnica que Argos *tem* rodou em 0,5%. `[CÓDIGO]` + `[PUBLICADO]` A técnica que o
estado da arte reporta como valendo +24% de dados de imagem não existe na ferramenta. Cinco linhas
de alto ganho da Fase C estão não executadas. **Este requisito é o que trava a conclusão.**

**3. Fundamento físico — SATISFEITO, e é o mais forte dos três.**
`[PUBLICADO]` (A7) Onde houve sobrescrita, a perda é definitiva: 1,4×10⁻²⁵⁸ para 1024 bits num drive
usado, e um JPEG tem ordens de grandeza mais que isso. NIST SP 800-88 Rev. 1 é a posição normativa
correspondente. Nenhuma técnica futura muda isso — não é limitação de ferramenta, é o meio.

**Conclusão da Fase E:** `[INFERIDO]` a afirmação de irrecuperabilidade **não pode ser feita hoje**,
e o que a bloqueia é o requisito 2, que é o único dos três inteiramente sob controle do usuário.
Requisito 3 já dá a certeza que ele pediu, mas condicionada: *se* sobrescrito, perdido para sempre,
sem apelação. O que falta é estabelecer o antecedente.

**"A ferramenta não achou" ≠ "não está lá".** Com o requisito 2 falhando desse tamanho, a distância
entre as duas proposições é grande, e nenhuma leitura do resultado atual autoriza tratá-las como
equivalentes.

---

## Fase F — playbook

Nenhum passo escreve na mídia de origem. Ordenados por (ganho × probabilidade) ÷ custo.

**Passo 0 — imagem forense. Bloqueia tudo o mais.**
`[MEDIDO]` `OPEN-WORK §4A`: 193 GB livres contra 1 TB necessários; o disco tem 10 anos e já reporta
98 regiões ilegíveis. `[PUBLICADO]` (A7) o uso continuado do sistema é justamente o que Wright *et
al.* apontam como o que "anula qualquer chance" — cada boot consome espaço não alocado. Adquirir um
disco externo de 1 TB+ e rodar `argos acquire` (C07) é a única ação que impede a evidência de
continuar sendo gasta. **Testa:** nenhuma hipótese; preserva a capacidade de testar todas.

**Passo 1 — interrogar o manifesto existente. Minutos, zero código.**
Consultar os registros por `width`/`height`, `camera_make`/`camera_model`, `taken`, e o campo
`lost_files`. `[MEDIDO]` 952 artefatos carregam `DateTimeOriginal` EXIF e as câmeras estão nomeadas
em `OPEN-WORK §1.4`. **Testa H3 e H4.** Se os nomes ou as datas do lote aparecerem, a busca acabou de
ficar dirigida em vez de cega.

**Passo 2 — caracterizar os sobreviventes. Minutos.**
Formato (`SOF0` vs `SOF2`), dimensões, contagem de extents, offsets, `same_size_neighbours`.
**Testa H5, H6, H7 de uma vez**, e produz o insumo do Passo 4.

**Passo 3 — reexecutar sem piso, na vizinhança. Horas.**
`--range` em torno dos offsets dos sobreviventes com `--min-long-side 0`. `[CÓDIGO]` `config.rs:265`
garante que os extents no manifesto localizam os bytes exatamente para uma reexecução com piso menor
— não é preciso varrer o disco de novo. **Testa H3 e H6.**

**Passo 4 — os sobreviventes como oráculo de texto-claro conhecido. É o ativo subaproveitado.**
`[PUBLICADO]` (A2) JpgScraper precisa de uma base de 3.269 modelos de câmera porque não sabe qual
câmera produziu o fragmento. Aqui se sabe. Dos menos de 10 arquivos extraem-se as tabelas DQT e DHT
exatas, a estrutura de APPn da câmera, e as strings EXIF de fabricante e modelo. Daí:
- (a) buscar essas sequências de bytes por toda a superfície → localiza **cabeçalhos irmãos** que a
  validação atual pode ter reprovado;
- (b) buscar os nomes de arquivo em **UTF-16LE** por toda a superfície, que é como o NTFS os grava →
  alcança registros `$MFT`, entradas `$I30`, `$UsnJrnl`, atalhos e bancos do OneDrive numa passada;
- (c) usar as tabelas conhecidas para decodificar fragmentos **sem cabeçalho** — o que a literatura
  de A2 faz estimando, aqui se faz sabendo.
`[INFERIDO]` (a) e (b) são buscas por assinatura, executáveis hoje sem alterar Argos. (c) exige o
item 3 da Fase C. **Testa H1 e H2**, e é a única linha que ataca a hipótese de sobrescrita
diretamente: se nenhuma DQT irmã aparece em lugar nenhum da superfície, isso é evidência positiva de
sobrescrita, não mera ausência de achado.

**Passo 5 — completar a busca de remontagem. ~15 h de máquina, sobre a imagem.**
`reassemble --from` sobre o manifesto, sem orçamento. `[MEDIDO]` `defects/07` estabelece que o
resultado é o mesmo para qualquer contagem de threads. **Testa H2.**

**Passo 6 — validar contra verdade-fundamental. Independente do disco.**
`[PUBLICADO]` (A8) DFRWS 2006/2007 e NIST CFReDS FC-01..FC-05. Fecha P9 e é o que converte "a
ferramenta não achou" em algo com taxa conhecida. Sem isso a Fase E nunca fecha o requisito 2, mesmo
que todas as técnicas sejam implementadas.

**O que devolve pixels:** passos 3, 4(c), 5, e um parser de thumbcache (Fase C item 4).
**O que devolve a lista do que se perdeu:** passos 1, 4(b), e OneDrive/`$Recycle.Bin`. `[INFERIDO]`
A segunda categoria não é consolação: nomes e datas convertem uma varredura cega numa busca dirigida,
e é o que torna o Passo 4 possível para além dos sobreviventes que já se tem.

---

## Fase G — autocrítica

**A condição de independência não foi satisfeita.** Esta auditoria foi executada pela mesma sessão
que redigiu o prompt e a tabela de lacunas da §2.1 dele. O prompt existe justamente para ser rodado
de fora. Esta execução deve ser tratada como uma primeira passada que reduz o custo da segunda, não
como o contraditório que a §0 pede. As Fases A e E são as menos contaminadas (literatura externa e
critério fixado a priori); a Fase C é a mais contaminada, porque herda a estrutura da §2.1.

**As três afirmações mais prováveis de estarem erradas:**

1. **Que H2 é a hipótese melhor sustentada.** Ela repousa em 0,5% de execução, o que estabelece
   *ignorância*, não presença. Uma busca que não rodou não é evidência de que há algo para achar.
   Se o Passo 2 mostrar que os sobreviventes têm um único extent cada, H2 perde sua base.
2. **Que os sobreviventes servem de oráculo (Passo 4).** Depende de os menos de 10 serem originais da
   câmera e não entradas de cache reprocessadas. Uma miniatura de thumbcache foi recodificada pelo
   Windows e **não** carrega as tabelas da câmera. Se o Passo 2 confirmar H6 para todos eles, o
   Passo 4 perde a maior parte do seu valor.
3. **Que a lacuna de 2 bytes de Garfinkel importa.** Marquei como `[DESCONHECIDO]` e mantenho, mas
   inclinei-me a tratá-la como possivelmente real sem ter lido a definição de gap do artigo.

**Onde usei `[INFERIDO]` e cabia `[DESCONHECIDO]`:** na leitura de que o perfil do disco é o de alta
fragmentação de Garfinkel (A1). O corpus dele é de discos de até 20 GB de meados dos anos 2000; a
extrapolação para um disco de 1 TB moderno é plausível e não medida. O próprio artigo diz que a
fragmentação cai com o tamanho do disco.

**Citações não verificadas:** a atribuição de autoria de "Forensic Analysis of Windows Thumbcache
files" (Quick & Tassone) e o ano/volume do "NTFS Data Tracker" foram obtidos por busca e **não** por
leitura do artigo. Os achados que atribuo a eles foram confirmados em fontes secundárias, não no
texto original. `[NÃO VERIFICADO]` — não sustentam conclusão sozinhos, e as conclusões da Fase C que
dependem deles (itens 4 e 5) devem ser reconfirmadas antes de virarem trabalho.

Uzun & Sencar 2015/2020, Garfinkel 2007, Wright *et al.* 2008, FiFTy 2021, NIST SP 800-88 Rev. 1,
DFRWS 2006/2007 e NIST CFReDS estão **verificados**: veículo, volume e ano confirmados, e no caso de
Wright e Garfinkel os números foram lidos do PDF do artigo, não de resumo.

**Condição de refutação da conclusão principal.** Concluí que a irrecuperabilidade não pode ser
afirmada. Mudaria de ideia se: os Passos 1–4 rodassem, nenhuma DQT irmã, nenhum nome em UTF-16LE,
nenhuma entrada `lost_files` e nenhum artefato sob o piso correspondessem ao lote, **e** a região dos
sobreviventes se mostrasse ocupada por conteúdo posterior datável. Isso satisfaria os requisitos 1 e
2 da Fase E, e o requisito 3 já converteria o resultado em irrecuperabilidade definitiva.

---

## Apêndice — referências

**Verificadas** (veículo, volume e ano confirmados; ✱ = números lidos do texto integral):

- ✱ Garfinkel, S. "Carving contiguous and fragmented files with fast object validation".
  *Digital Investigation* 4S (2007) S2–S12. DFRWS 2007.
- ✱ Wright, C., Kleiman, D., Sundhar R.S., S. "Overwriting Hard Drive Data: The Great Wiping
  Controversy". ICISS 2008, LNCS 5352, pp. 243–257. Springer.
- Uzun, E., Sencar, H.T. "Carving Orphaned JPEG File Fragments". *IEEE TIFS* 10(8):1549–1563, 2015.
- Uzun, E., Sencar, H.T. "JpgScraper: An Advanced Carver for JPEG Files". *IEEE TIFS* 15:1846–1857,
  2020. Código: `github.com/euzun/jpgscraper`, `github.com/euzun/jpeg-carver-csharp`.
- Mittal, G. *et al.* "FiFTy: Large-Scale File Fragment Type Identification Using Convolutional
  Neural Networks". *IEEE TIFS* 16, 2021. arXiv:1908.06148. Código: `github.com/mittalgovind/fifty`.
- NIST SP 800-88 Rev. 1, *Guidelines for Media Sanitization*, 2014.
- DFRWS 2006 e 2007 Carving Challenges. `github.com/dfrws/dfrws2007-challenge`.
- NIST CFReDS File Carving. `cfreds.nist.gov/FileCarving`.

**Não verificadas** (existência provável, texto integral não lido; não sustentam conclusão sozinhas):

- Quick, D., Tassone, C. *et al.* "Forensic Analysis of Windows Thumbcache files".
- "NTFS Data Tracker: Tracking file data history based on $LogFile". *FSI: Digital Investigation*.
- Literatura de artefatos OneDrive (ODL, SyncEngineDatabase) — fontes secundárias apenas.
- Gutmann, P. "Secure Deletion of Data from Magnetic and Solid-State Memory", USENIX Security 1996 —
  citado apenas como o alvo da refutação de Wright *et al.*, via a discussão dentro daquele artigo.

---

## Fase H — o que a execução do playbook mediu

Os Passos 1 e 2 da Fase F foram executados contra a sessão `argos-ntfs`. Isto é medição posterior:
as Fases D e E acima foram escritas antes e não foram reescritas, para que a distinção entre o que
se previu e o que se observou continue legível.

### H.1 A sessão

`[MEDIDO]` `ata-ST1000DM003-1CH162_S1DAZD8K`, 1.000.204.886.016 bytes. A corrida varreu
677.380.841.472 bytes começando em 300,653 GiB — exatamente o início do volume **NTFS residual** de
630,858 GiB, cluster de 4096 B, que o `residue` localizou. Foi uma varredura dirigida ao antigo
Windows, e a faixa foi coberta inteira: `carve`, `filesystem`, `validation`, `reassembly` e `report`
terminaram; o cancelamento ocorreu depois, durante a triagem.

`[MEDIDO]` **Os primeiros 300,653 GiB do disco ficaram fora desta corrida.** Nada nesta sessão diz
coisa alguma sobre eles.

`[MEDIDO]` 100.535 registros: 17.944 escritos, 82.591 omitidos sob o piso de 300 px. Confiança:
98.521 `contiguous-carve`, 1.899 `partial-or-thumbnail`, 70 `reassembled`, 45 `fs-metadata`.
Formato: 64.311 PNG, 36.224 JPEG. 519 carregam câmera, 589 carregam `taken`, 111 carregam nome
recuperado, 6.039 carregam `same_size_neighbours`.

### H.2 O achado estrutural

`[MEDIDO]` **Distribuição de extents: 1 → 100.434; 2 → 96; 3 → 5.** Apenas **101 de 100.535**
artefatos têm mais de um extent.

`[INFERIDO]` (de H.2 + H.3) O que este pipeline entrega neste meio é quase inteiramente contíguo.
Não é que os sobreviventes do lote sejam especiais — é que **o fragmentado está praticamente ausente
da saída**, e fragmentado é o que a literatura (A1) diz ser o estado de arquivos de interesse
forense num disco muito usado.

### H.3 A remontagem, medida

`[MEDIDO]` Do log da corrida: `reassembly began, 16321 items` em 10.253 s, `reassembly ended, 135
findings` em 62.880 s. **14,6 horas.** O contador chegou a 3.706 de 16.321.
`reassembly_attempted: 1876`, `reassembled: 135`.

| | Valor |
| --- | --- |
| pontos de fragmentação registrados | 17.217 |
| descartados sob o piso de 300 px | 9.418 |
| **cabeçalhos efetivamente na fila** | **7.799** |
| regiões planejadas | 723 |
| tentados em 14,6 h | 1.876 (**24,1%**) |
| recuperados | 135 |
| **aproveitamento por tentativa** | **7,2%** |
| fila inteira, na mesma taxa | **~61 h** |
| rendimento projetado da fila inteira | **~561 imagens** |

O `16321` do log não é uma contagem de itens: é `cabeçalhos × 2 + regiões`, porque cada cabeçalho
custa uma busca de lacuna e uma caminhada, e cada região custa uma leitura. `7.799 × 2 + 723 =
16.321` fecha exatamente, e `1.876 × 2 = 3.752` explica o contador ter parado em 3.706. Ver I.3.

`[INFERIDO]` (de H.3) A técnica **funciona neste disco** — 7,2% das tentativas produzem uma imagem,
com 0 fabricações pelo desenho do estágio. O que falta não é método, é fila consumida: três quartos
dos cabeçalhos elegíveis nunca foram tentados, e cerca de 560 recuperações seguem não reclamadas.

`[MEDIDO]` Dos 17.217 pontos de fragmentação registrados: 13.008 PNG, 4.209 JPEG, dos quais **920
declaram quadro ≥ 640×480**. Progresso de decodificação desses 920: mediana **10,5%**, 442 abaixo de
10%, 39 acima de 90%.

### H.4 Resolução das hipóteses da Fase D

- **H7 — RESOLVIDA, e reenquadra o resto.** `[MEDIDO]` 100.434 de 100.535 artefatos têm um extent.
  A propriedade que distingue os sobreviventes é serem contíguos.
- **H2 — CONFIRMADA como a restrição vinculante.** `[MEDIDO]` 24,1% dos cabeçalhos tentados, 7,2%
  de aproveitamento. É a hipótese mais bem sustentada e agora com número.
- **H3 — MAJORITARIAMENTE REFUTADA.** `[MEDIDO]` Dos 82.591 omitidos, apenas **92 carregam `taken`**
  e 36 uma câmera; as dimensões dominantes são 32×32 (8.628), 64×64 (7.725), 48×48 (4.773), 128×128
  (4.557), 16×16 (4.404). O conjunto omitido é cache e ícone, não fotografia — **o piso de 300 px
  está fazendo o trabalho certo**. Ressalva: `[MEDIDO]` **7.596 omitidos têm lado maior entre 250 e
  299 px**, encostados no piso, e esses merecem uma olhada dirigida.
- **H5 — REFUTADA quanto a formato.** `[MEDIDO]` Os 37 artefatos da era 2003–2012 são JPEG e
  decodificam.
- **H6 — CONFIRMADA para parte do conjunto.** `[MEDIDO]` Vários FE170 em 640×480 vêm como
  `partial-or-thumbnail`, contra um único FE170 em 2816×2112 nativo — a resolução da câmera.
- **H1 — segue `[DESCONHECIDO]`, mas agora delimitada.** A faixa não varrida (300,65 GiB iniciais)
  não autoriza nenhuma leitura de ausência.

### H.5 Dois achados que não estavam nas hipóteses

`[MEDIDO]` **`journal_deletions: 0`.** O `$UsnJrnl` não produziu uma única exclusão num volume NTFS
residual de 630 GiB. `[CÓDIGO]` `OPEN-WORK §3.9a` já registra que o nome do stream e o layout
`USN_RECORD_V2` são não verificados contra mídia real, porque todo fixture escreve o que as
constantes dizem. `[INFERIDO]` Este é exatamente o caso que aquele item prevê, e o resultado é
compatível com as duas explicações — o journal não sobreviveu, ou o leitor não funciona em mídia
real. **É um teste discriminante que o projeto nunca rodou e que custa uma leitura dirigida.**

`[MEDIDO]` **45 artefatos em `fs-metadata` e 111 com nome recuperado**, de um volume NTFS de 630 GiB.
`[INFERIDO]` A via de metadados está quase morta numa segunda corrida independente, o que reforça o
diagnóstico de `OPEN-WORK §1.5` em vez de o atribuir àquela corrida específica. `unattributed_residue`
foi 0 aqui, então **não é o `§3.8`** — as âncoras existiam; o que não apareceu foram os registros.

`[MEDIDO]` 11.558 volumes localizados, **11.551 ext4** contra 7 NTFS, num disco cuja faixa varrida é
um volume NTFS. É a taxa de falso positivo de `OPEN-WORK §3.6` reproduzida.

### H.6 A mudança que a medição motivou

`[CÓDIGO]` A fila de remontagem era ordenada por `Broken::progress()` — a **fração** do quadro
decodificada. `[MEDIDO]` Com mediana de 10,5% entre os 920 quadros grandes e 442 deles abaixo de
10%, essa chave manda as fotografias para o fim de uma fila que o relógio nunca esvazia: um quadro
400×400 três quartos decodificado (594 MCUs) precede um 2816×2112 um décimo decodificado (2.300
MCUs).

O racional escrito no próprio spec já dizia o contrário — "a frame the decoder walked **thousands of
MCUs** into is a photograph; one it walked **three** into is a signature that landed on plausible
bytes" — que é uma afirmação sobre **unidades absolutas**, não sobre fração. A implementação não
correspondia ao seu spec.

**A ordenação agora é pelo número absoluto de unidades decodificadas.** Spec e código foram
atualizados juntos (`A-ALGORITHM-FROM-SPEC`), com dois testes: um prova que a fotografia um décimo
decodificada precede o quadro pequeno quase completo — ambos acima do piso, de modo que só a ordem
os separa — e outro que um formato sem contagem de unidades ordena por último em vez de empatar.

`[INFERIDO]` Isto não acelera a busca e não muda o que ela é capaz de recuperar. Muda **qual** 22,7%
da fila um relógio compra, que — dado H.3 — é a variável que decide o resultado.

### H.7 O que fazer agora, concretamente

```
argos reassemble --from ~/Imagens/argos-ntfs /dev/disk/by-id/ata-ST1000DM003-1CH162_S1DAZD8K \
      --out ~/Imagens/argos-ntfs-remontagem --reassembly-budget 0 --min-long-side 0
```

`--reassembly-budget 0` procura todo candidato pelo tempo que for preciso; `--min-long-side 0`
escreve tudo. `[MEDIDO]` Custo esperado ~61 h na taxa medida; rendimento projetado ~561 imagens.
Com a nova ordenação, as fotografias de quadro grande vêm primeiro, então uma corrida interrompida
em 24 h já entrega a parte que interessa — o que antes não era verdade.

**Antes disso**, porém, vale o Passo 0: `[MEDIDO]` 96 regiões ilegíveis nesta corrida, disco de dez
anos, sem imagem forense. 127 horas de leitura contínua sobre essa mídia é precisamente o risco que
o `OPEN-WORK §4A` descreve.

`[DESCONHECIDO]` O que há nos 300,65 GiB iniciais. Encerraria: uma varredura `--range 0..300653000000`.

---

## Fase I — o ranking revisto pela evidência

A Fase C foi escrita antes de qualquer medição. A Fase H a contradiz em pontos que importam. Esta
seção é a correção; a Fase C fica como estava, para que se veja o que a medição mudou.

### I.1 A via de metadados está morta para este lote

`[MEDIDO]` Os 111 artefatos que a etapa de sistema de arquivos entregou com nome recuperado são:
47 nomes que são hashes SHA-1 de 40 caracteres sem extensão (60×60, 74×74, 100×100, 264×198 —
cache de aplicativo), 58 PNG de interface do tipo `A12_Checkmark_White@1x.png`, cada um em duas
cópias, e um resto de `CiPT0000.001` (índice de busca do Windows), `.sqlite-shm`, `.log`, `.js`,
`.xml`, `.kdc`.

**Nenhuma fotografia.** Nenhum `IMG_*`, nenhum `DSC*`, nada de câmera ou de scanner.

`[INFERIDO]` (de I.1 + H.5) Os registros `$MFT` das fotografias foram reciclados pelo churn que
sobreviveu — arquivos de cache, escritos até o fim da vida daquele Windows. Combinado com
`journal_deletions: 0`, a conclusão é que **a evidência de metadados deste lote não existe mais no
volume**, e não que a ferramenta não a leia.

**O que isso rebaixa:**

- **Item 5 (`$LogFile`) — de Alto para Baixo.** O `$LogFile` é circular e de tamanho fixo: guarda as
  transações mais recentes. Pelo mesmo argumento que explica o conteúdo do `$MFT`, o que ele
  guardaria é cache, não uma exclusão de ~2022. Continua correto implementá-lo; deixa de ser uma
  rota para este lote.
- **Itens 7 (OneDrive) e 8 (`$Recycle.Bin`) — de Médio para Baixo.** Ambos são arquivos dentro do
  sistema de arquivos, sujeitos à mesma reciclagem. Devolveriam a lista do que se perdeu, que
  continua valendo, mas com a mesma probabilidade de sobrevivência que os registros acima.
- **Item 4 (thumbcache) — de Alto para Médio.** As entradas de cache **já estão sendo carveadas**:
  são fluxos JPEG/PNG completos e aparecem entre os 82.591 abaixo do piso. Um parser acrescentaria o
  nome e o caminho originais, não pixels novos.
- **Item 9 — refutado.** `unattributed_residue: 0`.

### I.2 O que sobra

`[INFERIDO]` **Carving e remontagem são a rota inteira.** Não porque sejam as melhores, mas porque
são as únicas cuja matéria-prima — os clusters de dados — ainda pode estar no disco. Registros de
metadados são pequenos e ficam numa zona dedicada, reciclada por escrita de arquivo; clusters de
dados de 1–5 MB são reciclados noutro ritmo. As duas coisas não morrem juntas, e a morte de uma não
prova a da outra.

Ranking revisto, apenas do que continua vivo:

| # | Ação | Estado | Custo em passagens de disco |
| --- | --- | --- | --- |
| 1 | **Completar a busca de remontagem** | pronta para rodar | **nenhuma passagem completa** — `reassemble --from` lê só as regiões dos pontos |
| 2 | Carving de fragmento órfão sem cabeçalho (item 3) | `restart_points()` existe, não ligado | pega carona no mesmo estágio |
| 3 | TIFF (item 11) | ausente | exige varredura nova |
| 4 | Validação contra DFRWS/CFReDS (item 6) | P9 pendente | nenhuma — não toca o disco |
| 5 | Confirmação da âncora ext4 (item 10) | `§3.6` | exige varredura nova |

`[INFERIDO]` Sem imagem forense, o recurso escasso é **passagem pelo disco**, não tempo de máquina.
Isso põe o item 1 sozinho no topo: é o único de valor medido que não custa uma passagem. Tudo que
exige varredura nova deve ser acumulado e gasto de uma vez só, numa passagem, quando houver algo que
justifique.

### I.3 Retratação: os contadores fecham, e o defeito é outro

Uma versão anterior desta seção afirmou que três contadores da mesma corrida não reconciliavam.
**Isso estava errado.** `[CÓDIGO]` `crates/argos_engine/src/pipeline.rs`, `Counter::start` na entrada
de `reassemble_broken`, dá ao contador o total `broken.len() * 2 + plans.len()` — cada cabeçalho
custa uma busca de lacuna e uma caminhada, e cada região custa uma leitura. Com
`skipped_small = 9418` sobre 17.217 pontos, os cabeçalhos na fila são 7.799, e
**`7.799 × 2 + 723 = 16.321`**, exatamente o número do log. `1.876 × 2 = 3.752` explica o contador
ter parado em 3.706. Nada discorda.

**O defeito real é de relato, não de contagem.** `[CÓDIGO]` A linha de progresso nomeia a unidade
como `items`, e o número não é uma contagem de coisa nenhuma que um leitor possa nomear: é uma soma
de três grandezas de naturezas diferentes. `docs/CAPABILITIES.md` C42 promete progresso "with the
unit named so a candidate count is never read as a byte count" — a unidade está nomeada e mesmo
assim engana, porque o nome não corresponde ao que é contado.

`[MEDIDO]` O custo disso é aferível: esta auditoria leu `3706/16321` como pontos de fragmentação e
publicou três números errados — 11,5% de fila atendida em vez de 24,1%, 127 h em vez de 61 h, e
~1.174 imagens em vez de ~561. Um relato que leva um leitor atento a uma conclusão errada é
exatamente a falha que `A-CONFIDENCE-HONEST` existe para impedir.

---

## Fase J — o que foi implementado

### J.1 Item 2 — a unidade de progresso da remontagem

`[CÓDIGO]` `Counter` passou a carregar a unidade que conta, e a remontagem declara `Unit::Steps` em
vez de `Unit::Items`. A linha impressa passa a nomear a unidade (`3706 of 16321 steps`), o que retira
dela o convite à aritmética que ela não suporta. O número em si não mudou e não podia mudar: ele é
`cabeçalhos × 2 + regiões` porque cada cabeçalho custa uma busca de lacuna e uma caminhada, e reduzi-lo
a cabeçalhos exigiria ou parar o contador durante uma fase inteira ou contar cada cabeçalho duas
vezes — e `docs/defects/07` fixou justamente que a fase não pode ficar muda.

Teste: `reassembly_counts_steps_and_does_not_offer_them_as_a_candidate_count` planta uma fotografia
fragmentada, confirma que a etapa se anuncia em `steps`, que o total anunciado é maior que
`reassembly_attempted` — a premissa que torna a leitura errada possível — e que a unidade não muda no
meio da etapa. O número que significa cabeçalhos continua sendo `reassembly_attempted`, no manifesto.

### J.2 Item 1 — recall contra corpus publicado

`[CÓDIGO]` `crates/argos_engine/tests/corpus_recall.rs`. Lê pares imagem/gabarito de
`ARGOS_CORPUS_DIR` (`caso.raw` ao lado de `caso.sha256`, no formato do `sha256sum`), roda o pipeline
completo sobre cada imagem sem piso de tamanho e sem orçamento de remontagem, e reporta quantos dos
arquivos conhecidos voltaram **byte a byte** — um prefixo parcial da fotografia certa não conta como
recuperação dela. Ausência da variável não é falha: é corpus não fornecido, e o teste diz o que
precisaria.

Os corpora que isso mede são os desafios de carving DFRWS 2006 e 2007 e os casos NIST `CFReDS`
FC-01..FC-05, cujos cenários fragmentados e de cluster não alinhado são exatamente os que as fixtures
deste projeto não propõem.

**O instrumento é auto-verificado**, que é o que separa uma medição de uma afirmação: um teste planta
duas fotografias, remove uma do meio antes da varredura, e exige que o harness credite a que está lá
e nomeie a que não está — as duas direções, porque um instrumento que acha tudo e um que não acha nada
são igualmente inúteis. Um terceiro teste fixa a leitura do formato `sha256sum` nos dois modos e exige
que um gabarito malformado seja reportado em vez de descartado, porque uma lista de respostas
silenciosamente encurtada infla o recall.

Verificado de ponta a ponta contra um corpus sintético: `synthetic 66% 2 of 3 known files`, com
`missed never-planted.jpg`.

`[DESCONHECIDO]` **O recall real de Argos contra gabarito externo.** O mecanismo existe; os dados não
foram fornecidos. Encerraria: baixar um dos corpora acima e apontar `ARGOS_CORPUS_DIR` para ele.
Enquanto isso não acontecer, os 7,2% de aproveitamento medidos em H.3 continuam sem referência — não
se sabe se são saudáveis ou se escondem um defeito, e o requisito 2 da Fase E permanece insatisfeito.

`cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` e `cargo test --workspace`
(307 testes) passam.

---

## Fase K — itens 3 e 4

### K.1 O que foi construído

`[CÓDIGO]` `crates/argos_carve/src/reference.rs`. Um `Reference` é o intervalo de bytes de `SOI` até
o fim do `SOS` de uma imagem sobrevivente, **copiado literalmente e nunca reescrito** — reescrever é
uma chance de gravar uma tabela diferente da que a câmera gravou, e que ele não faça isso é o valor
inteiro da técnica. `Reference::graft` produz prefixo + dados entrópicos do órfão + `EOI`.

Um quadro progressivo, aritmético, sem perdas ou hierárquico é **recusado** (`Fault::NotSequential`):
cada varredura de um progressivo carrega os próprios parâmetros, então o prefixo de uma não
decodifica os dados de outra, e emprestá-lo produziria absurdo confiante.

### K.2 A medição que importa

`[MEDIDO]` `crates/argos_carve/tests/reference.rs`, sete testes:

- **Identidade**: os dados entrópicos de um arquivo enxertados no cabeçalho dele mesmo reproduzem o
  arquivo **byte a byte** — o que prova que o corte do prefixo está no lugar certo.
- **Irmãos**: o cabeçalho da foto A decodifica os dados entrópicos da foto B, mesma câmera, mesmo
  tamanho.
- **Órfão**: metade final dos dados entrópicos de uma fotografia de 320×240, **sem cabeçalho e sem
  nada antes dela**, entrada no primeiro marcador de reinício e enxertada — **decodifica para imagem
  real** (`roughness() > 0`, não uma cor chapada).
- **O limite, fixado como teste**: uma fotografia codificada sem `DRI` não oferece ponto de reentrada
  nenhum, e nenhum pode ser inventado para ela.

`[PUBLICADO]` (A2) É a técnica de Uzun & Sencar. A dificuldade central publicada — estimar os
parâmetros de codificação a partir de uma base de 3.269 modelos de câmera — **não existe neste caso**:
os menos de 10 sobreviventes fornecem as tabelas exatas.

`[CÓDIGO]` Alvo de fuzz `reference_read` acrescentado e registrado na lista da CI
(`.github/workflows/ci.yml`), conforme `A-FUZZ-EVERY-PARSER`.

### K.3 O que ainda não existe

`[CÓDIGO]` **Nada disso está ligado ao pipeline nem à CLI.** Nenhum estágio oferece um órfão ao
enxerto e nenhuma flag nomeia um arquivo de referência. `docs/CAPABILITIES.md` **não** ganhou linha
nova, porque aquele documento é um contrato sobre o que é alcançável, e isto ainda não é.

O que falta é a fiação e o relato, e o relato é a parte delicada: um enxerto é **pixels num
recipiente que esta ferramenta construiu**, nunca um arquivo recuperado. O quadro declara as
dimensões da referência, a posição da faixa dentro dele é desconhecida, e aqueles bytes naquela ordem
nunca existiram no disco. Precisa do tier mais fraco e de um campo de proveniência nomeando a
referência (`A-CONFIDENCE-HONEST`). Registrado em `OPEN-WORK §3.9h`.

`cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` e `cargo test --workspace`
(**314 testes**) passam.
