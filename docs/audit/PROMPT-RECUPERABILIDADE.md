# Auditoria de recuperabilidade — lote de fotografias excluído no Windows

Prompt para uma sessão de auditoria independente. Colar integralmente, a partir da §0, numa sessão
nova com acesso a busca web e ao repositório. O contexto abaixo da §2 é injetado de propósito: a
auditoria começa do estado real medido e não gasta esforço remapeando o que já está estabelecido.

---

## 0. Papel

Você é um perito forense digital revisando o trabalho de outro perito. O autor da ferramenta
auditada é a pessoa que perdeu os dados, então ele é a última pessoa capaz de avaliá-la sem viés.
Sua função é ser o contraditório: não confortar, não presumir competência, não presumir
incompetência. A pergunta a responder tem consequência emocional real, e por isso a única resposta
aceitável é a defensável.

Você **não** deve produzir uma opinião. Deve produzir um dossiê em que cada afirmação carrega sua
própria proveniência, de modo que o leitor possa rejeitar qualquer conclusão sua sem rejeitar o
resto.

As duas perguntas a responder, literalmente:

1. As fotografias desse lote ainda podem ser recuperadas?
2. Argos usa o que existe de estado da arte na bibliografia de recuperação de imagens profundamente
   apagadas, e deixa passar algum resquício que levaria a elas?

## 1. Contrato de evidência — regra que invalida a saída se quebrada

Toda proposição afirmativa do relatório carrega exatamente uma etiqueta:

- `[MEDIDO]` — número produzido por uma execução, manifesto, teste ou benchmark **deste
  repositório**. Cite arquivo e o valor. Se o número não existe, não invente a medição: use
  `[DESCONHECIDO]`.
- `[CÓDIGO]` — afirmação sobre o que o código faz ou não faz. Cite `caminho:linha`. Uma afirmação de
  ausência exige o comando de busca que a sustenta.
- `[PUBLICADO]` — afirmação retirada da literatura. Cite autores, título, veículo e ano, **e o
  achado específico** (número, taxa, condição experimental). Uma citação sem achado específico não
  sustenta nada.
- `[INFERIDO]` — dedução. Liste as premissas etiquetadas de que ela decorre. Uma inferência a
  partir de outra inferência deve dizê-lo.
- `[DESCONHECIDO]` — não estabelecível com o disponível. Obrigatório declarar **qual medição
  encerraria a questão**.

Regras adicionais, todas bloqueantes:

1. **Nenhuma citação não verificada.** Se você não conseguir confirmar que um trabalho existe com
   aquele título, veículo e ano, marque `[NÃO VERIFICADO]` e ele **não pode** sustentar conclusão
   alguma. Prefira falhar em encontrar a inventar. Fabricar uma referência aqui é a pior falha
   possível desta tarefa.
2. **Sem hedge portante.** "Provavelmente", "geralmente", "costuma-se", "é comum" não podem ser o
   sustentáculo de uma conclusão. Se a evidência é fraca, a conclusão é `[DESCONHECIDO]`.
3. **Prosa sem etiqueta é saída inválida.** Exceto títulos e conectivos.
4. **Números do repositório vencem seus modelos.** Se um raciocínio seu contradiz um número medido,
   o raciocínio está errado. `docs/defects/07-a-hypothesis-costs-what-it-decodes-through.md` é o
   precedente: um modelo reproduziu o total do estágio dentro de −10,2% cancelando dois erros
   opostos, e estava errado nas duas metades.

## 2. O caso

- Mídia: **HDD magnético**. Sem TRIM, sem FTL, sem remapeamento por wear leveling — o que está no
  LBA *N* é o que o host escreveu ali.
- Perda: exclusão acidental **dentro do Windows** (não por formatação), ~2022. Pasta sincronizada
  com OneDrive; as cópias na nuvem foram excluídas junto, no mesmo momento.
- O lote é **homogêneo**: mesmas circunstâncias, mesmo instante, mesmo sistema de arquivos.
- **Houve formatações/reinstalações de sistema operacional depois** da exclusão.
- Hoje: varredura ao vivo do dispositivo, **sem imagem forense**; 193 GB livres contra 1 TB
  necessários; o disco tem ~10 anos e reporta 98 regiões ilegíveis.
- Resultado atual: milhares de imagens recuperadas, **menos de 10 pertencentes ao lote**.
- Sobreviventes: esses menos de 10 arquivos existem e estão em mãos.

### 2.1 Estado verificado da ferramenta

Levantado por leitura de código, não por leitura da documentação. Trate como ponto de partida a
confirmar, não como verdade.

**Implementado e exercitado** — `$MFT` (inclusive `$MFT` fragmentado e `$ATTRIBUTE_LIST` residente),
data runs não-residentes com buracos esparsos, varredura de registros `FILE` órfãos, `$I30` index
slack (só nomes), `$UsnJrnl:$J` (nomes e momento da exclusão), residue sweep de volumes anteriores,
carving JPEG/PNG com máquinas de estado reais, miniaturas EXIF, bifragment gap carving, caminho
guloso para *n* fragmentos com oráculo de decodificação entrópica, classificação de blocos por
entropia. Medido em `crates/argos_carve/tests/recovery_rate.rs`: 87% em dois e três fragmentos, 25%
em quatro, **0 fabricações** em seis padrões.

**Ausente, confirmado no código:**

| Lacuna | Onde | Relevância para este caso |
| --- | --- | --- |
| `$LogFile` inteiramente ausente — sem restart area, sem LSN, sem redo/undo | busca global: zero ocorrências | Alta — guarda cópias íntegras de registros `$MFT` com runlists `$DATA` |
| Restart markers calculados e **descartados** | `crates/argos_carve/src/reassemble.rs:1447`, `OPEN-WORK §3.3` | Alta — é a base do carving de fragmentos órfãos |
| JPEG progressivo e aritmético não remontam (`ScanStop::Unsupported`) | `crates/argos_carve/src/mcu.rs:421-422` | Média |
| Só JPEG e PNG são carveados; sem TIFF | `crates/argos_carve/src/lib.rs`, `OPEN-WORK §3.9e` | Alta — três das "câmeras" do corpus são scanners de mesa |
| ADS de usuário nunca recuperados; sem LZNT1/WOF; sem `$MFTMirr` | `crates/argos_fs/src/ntfs.rs` | Média |
| Registros órfãos assumem 1024 bytes (`DEFAULT_RECORD_SIZE`) | `crates/argos_fs/src/ntfs.rs` | Média |
| Volume cujas âncoras sumiram todas permanece inalcançável | `OPEN-WORK §3.8` — **não implementado** | **Crítica** |
| Residue ext4 sem confirmação: 15.157 falsos contra 29 NTFS | `OPEN-WORK §3.6` | Alta — polui todo o estágio |
| HPA/DCO não endereçados **e não declarados** | `crates/argos_device/src/device/linux.rs`, `OPEN-WORK §3.9d` | Média |
| `part.rs` fixa `SECTOR = 512` | `crates/argos_fs/src/part.rs:31` | Baixa — decisão documentada, não descuido |
| Zero parsers de: thumbcache, `$Recycle.Bin` `$I`/`$R`, OneDrive (`.odl`, SyncEngine DB), `Windows.edb`, `hiberfil.sys`, `pagefile.sys` | busca global | **Crítica** — é onde vive a evidência de uma exclusão *dentro* do Windows |
| **P9 pendente**: nenhuma validação contra corpus com verdade-fundamental pública | `DEVELOPMENT-PLAN §8` | **Crítica** — o recall nunca foi medido contra nada externo |

**Números da corrida de 1 TB** (`OPEN-WORK §1`), que são o diagnóstico mais forte disponível:
348.361 registros no manifesto / 47.658 escritos / **300.703 omitidos sob o piso de 300 px** /
388.301 assinaturas reprovadas na validação / 50.355 pontos de fragmentação localizados /
**remontagem: 254 tentados, 3 recuperados, teto de 2 h atingido** / **1.512 listas de runs órfãos
descartadas** / 91 recuperações por metadados de sistema de arquivos / 15.186 volumes localizados,
**nenhum atual**.

`docs/defects/07`: nas regiões densas em fotografias uma hipótese custa **580 µs** contra 2,5 µs em
ruído. O estágio é mais lento exatamente onde está o alvo. Depois da paralelização, ~15 h para
esvaziar a fila.

**Bloqueador declarado** (`OPEN-WORK §4A`): não existe imagem forense, e o disco de origem segue em
uso ao vivo.

## 3. Fase A — levantamento cego da literatura

**Execute esta fase antes de ler qualquer código.** O objetivo é evitar ancoragem: se você mapear a
ferramenta primeiro, vai enumerar o estado da arte como "o que a ferramenta faz, mais um pouco".
A §2.1 acima é contexto do caso, não a pauta da busca — não deixe que ela determine o que você
procura.

Produza um catálogo do estado da arte publicado, com `[PUBLICADO]` em cada linha, cobrindo no
mínimo:

**A1. Carving e remontagem de fragmentos.** Validação de objeto e bifragment gap carving; detecção
de ponto de fragmentação; caminhos gulosos para *n* fragmentos; medidas de custo de junção; a
evolução do campo em surveys; medições publicadas de fragmentação real em NTFS. Para cada técnica:
o que recupera, quais pré-requisitos exige, que recall e precisão foram reportados, **sobre qual
corpus**.

**A2. Fragmentos órfãos e arquivos sem cabeçalho.** Recuperação de JPEG cujo cabeçalho não existe
mais; estimativa dos parâmetros de decodificação a partir do próprio fragmento; uso de marcadores
de reinício como pontos de reentrada independentes. Esta subseção é a mais importante da fase —
declare explicitamente o estado da arte e seus limites medidos.

**A3. Classificação de fragmentos.** Métodos estatísticos, métodos por compressão, e os métodos
baseados em redes neurais que hoje lideram os benchmarks. Reporte a acurácia comparada e o custo
computacional.

**A4. Artefatos NTFS além do `$MFT`.** `$LogFile` (o que exatamente ele guarda de um arquivo
excluído, e por quanto tempo), `$UsnJrnl`, `$I30` index slack, `$MFTMirr`, ADS, arquivos
comprimidos. Para cada um: qual evidência de um arquivo excluído ele preserva, e se preserva
**conteúdo** ou apenas **metadado**.

**A5. Artefatos do Windows que guardam pixels independentemente do arquivo original.** Caches de
miniatura do Explorer, caches de aplicativos de fotos, índice de busca, `$Recycle.Bin`, cópias de
sombra de volume, `hiberfil.sys`, `pagefile.sys`. Para cada um: que resolução preserva, onde vive,
e quanto sobrevive a uma reinstalação de sistema operacional.

**A6. Artefatos do OneDrive.** Bancos do motor de sincronização e logs de diagnóstico; que
metadados de arquivos removidos persistem localmente; janelas de retenção do lado servidor. Separe
com clareza **o que devolve pixels** do que devolve **apenas a lista do que foi perdido** — a
segunda coisa tem valor próprio e não deve ser descartada.

**A7. A física do overwrite em mídia magnética.** Este é o ponto que decide a pergunta central.
Cubra a origem da tese de recuperação de dados sobrescritos por microscopia de força magnética, o
estudo empírico que a testou, e a posição do órgão de normalização sobre quantas passagens de
sobrescrita bastam. Reporte as **probabilidades por bit e por palavra** efetivamente medidas, não a
conclusão qualitativa. Declare explicitamente o que isso permite afirmar e o que não permite.

**A8. Corpora de validação com verdade-fundamental.** Os desafios de carving da comunidade e os
conjuntos de referência publicados por órgão de normalização, incluindo os voltados a recuperação
de arquivo excluído e a carving. Para cada um: o que contém, se a verdade-fundamental é pública, e
o que uma ferramenta precisa reportar para ser pontuada.

## 4. Fase B — o que Argos realmente faz

Leia `docs/CAPABILITIES.md`, `docs/OPEN-WORK.md`, `docs/DEVELOPMENT-PLAN.md §3` e `docs/defects/`.

**Não confie nesses documentos.** Eles são bem escritos, o que é exatamente o que torna perigoso
aceitá-los. Para cada capacidade declarada, confirme no código com `[CÓDIGO]` e `caminho:linha`, e
sinalize toda divergência entre o declarado e o implementado. `CAPABILITIES.md` se declara um
contrato; teste-o.

Preste atenção particular a:

- O que os testes de fixture provam de fato. Se um fixture é construído pelas mesmas constantes que
  o leitor consome, o teste prova que leitor e fixture concordam **e nada mais** — o próprio
  repositório admite isso para `$UsnJrnl` em `OPEN-WORK §3.9a`. Ache os outros casos.
- Onde um estágio tem teto de tempo, orçamento ou limite de contagem, e o que acontece com o que
  fica fora dele.
- Onde um piso de tamanho, uma etiqueta de triagem ou uma deduplicação decide o que **não** é
  escrito em disco.

## 5. Fase C — matriz de lacunas

Cruze A contra B. Uma linha por técnica publicada:

| Técnica | Fonte | Status em Argos | Recall que a literatura atribui | Ganho **neste caso** | Custo |

A coluna "ganho neste caso" é a que importa e é a mais fácil de preencher preguiçosamente. Ela exige
raciocinar sobre o perfil concreto: HDD, JPEGs de câmera e de scanner, exclusão de usuário dentro do
NTFS, formatações posteriores, ~5 anos de uso. Uma técnica de ponta que não se aplica a este perfil
deve ser marcada como não aplicável **e justificada** — isso é resultado, não omissão.

Ordene por (ganho × probabilidade de aplicar-se) ÷ custo. Declare qual é a **única** mudança que
você faria se pudesse fazer só uma.

## 6. Fase D — por que apenas menos de 10 de um lote homogêneo

Este é o sinal forense mais informativo do caso e deve ser tratado como tal: um lote gravado junto e
excluído junto sobreviveria quase todo ou quase nada — o "quase nada, com um resto" pede explicação
mecânica.

Enumere hipóteses concorrentes. Para **cada uma**, declare a **medição discriminante**: o que
observar no manifesto, no disco ou num experimento que a confirmaria ou a refutaria. Uma hipótese
sem medição discriminante não entra na lista.

Comece por estas — testadas, não assumidas — e acrescente as suas:

- **H1 — A região foi sobrescrita.** Formatações posteriores realocaram o espaço. Discriminante: o
  que ocupa hoje os offsets vizinhos aos sobreviventes, e a idade e origem daquele conteúdo.
- **H2 — Estão presentes e fragmentados, e a busca nunca chegou neles.** `OPEN-WORK §1` registra 254
  de 50.355 pontos atendidos, e `defects/07` mede que uma hipótese custa duas ordens de grandeza
  mais em região densa em fotografias — isto é, o estágio é mais lento exatamente onde o alvo está.
  Discriminante: rodar `reassemble --from` sobre o manifesto existente, sem orçamento, restrito por
  `--range` à vizinhança dos sobreviventes.
- **H3 — Foram recuperados e não foram escritos.** 300.703 artefatos ficaram sob o piso de 300 px e
  existem no manifesto com extents e digest. Discriminante: consultar o manifesto por dimensão,
  câmera e data, ignorando o diretório de saída.
- **H4 — Os runlists existem e foram descartados.** 1.512 regiões de registros `FILE` órfãos foram
  jogadas fora por falta de volume ao qual ancorar (`OPEN-WORK §3.8`, não implementado).
  Discriminante: o campo `lost_files` do manifesto — os nomes do lote aparecem lá?
- **H5 — Não são JPEG nem PNG baseline.** Progressivo, aritmético, TIFF de scanner, HEIC ou RAW são
  invisíveis ou não-remontáveis. Discriminante: o formato exato dos sobreviventes.
- **H6 — A evidência sobrevivente não é o arquivo, é o cache.** `defects/02` estabelece que caches
  de miniatura sobrevivem às fotografias que descrevem, porque arquivos grandes são sobrescritos
  primeiro. Discriminante: `same_size_neighbours` e as dimensões dos sobreviventes.
- **H7 — Os sobreviventes têm uma propriedade que os demais não têm.** Menores, contíguos, ou em
  região fria do disco. Discriminante: comparar tamanho, contagem de extents e offset dos
  sobreviventes contra a distribuição geral do manifesto.

Conclua com o **ranking de hipóteses por evidência**, não por plausibilidade. Se duas explicam
igualmente bem, diga isso.

## 7. Fase E — o teste de irrecuperabilidade

Defina, **antes de olhar o resultado**, o que licenciaria a afirmação "estas fotografias não podem
mais ser recuperadas". Depois verifique se está satisfeito. A ordem importa: um critério definido
depois do resultado é um critério ajustado ao resultado.

O critério combina três coisas, e a falha de qualquer uma o invalida:

1. **Contabilidade de cobertura.** Todo setor endereçável foi lido, ou foi declarado ilegível.
   Argos já registra `coverage` e `unreadable` no manifesto. Inclua explicitamente o que está fora
   da superfície visível ao host: HPA/DCO não são endereçados nem declarados (`OPEN-WORK §3.9d`), o
   que é um buraco no argumento de cobertura e precisa ser tratado como tal.
2. **Exaustão de técnica.** Nenhuma técnica publicada aplicável ao perfil permanece não tentada. É a
   Fase C que preenche isto, e enquanto houver linha de alto ganho não executada, o argumento de
   irrecuperabilidade **não pode ser feito**.
3. **Fundamento físico.** O que a literatura de A7 permite afirmar sobre bytes efetivamente
   sobrescritos em mídia magnética, com os números medidos, e o que ela **não** permite afirmar.

Declare com precisão qual dos três está satisfeito hoje e qual não está. Se o argumento não pode ser
feito, diga que não pode ser feito — isso é um resultado honesto e mais útil que um veredito.

Trate separadamente, e não confunda: **"a ferramenta não achou"** e **"não está lá"** são
proposições diferentes, e a primeira só se aproxima da segunda na medida em que 1–3 estiverem
satisfeitos.

## 8. Fase F — playbook operacional

Sequência concreta e ordenada para esta mídia. Cada passo declara: o que executar, o que espera
encontrar, quanto custa, o que fazer com o resultado, e **qual hipótese da Fase D ele testa**.

Restrições que ordenam a lista:

- **Nada que escreva na mídia de origem.** Nenhum passo do playbook pode fazê-lo, e você não deve
  sugerir alternativa que o faça.
- **A ausência de imagem forense é o bloqueador de tudo** (`OPEN-WORK §4A`). Cada leitura do disco
  ao vivo desgasta uma mídia de 10 anos com 98 regiões já ilegíveis, e cada boot do sistema consome
  espaço não alocado. Trate a aquisição como passo zero e enderece o problema de espaço.
- **Os sobreviventes são um oráculo de texto-claro conhecido**, e este é provavelmente o ativo mais
  subutilizado do caso. Deles extraem-se tabelas de quantização e de Huffman exatas, a estrutura de
  cabeçalho da câmera, strings EXIF de fabricante e modelo, e o padrão de nomes de arquivo. Avalie
  explicitamente:
  - (a) buscar essas assinaturas byte a byte por toda a superfície, para achar cabeçalhos irmãos;
  - (b) buscar os nomes em UTF-16LE por toda a superfície, que é como o NTFS os grava, alcançando
    registros `$MFT`, entradas `$I30`, `$UsnJrnl`, atalhos e bancos do OneDrive de uma vez;
  - (c) usar as tabelas conhecidas para decodificar fragmentos órfãos — a técnica publicada de A2
    precisa **estimar** esses parâmetros, e aqui eles são **conhecidos**, o que muda a natureza do
    problema. Diga se isso procede e o que a literatura sustenta.
- Distinga passos que devolvem **pixels** de passos que devolvem **a lista do que se perdeu**. A
  segunda categoria tem valor próprio: dá nomes, datas e contagem, e delimita a busca.

## 9. Fase G — autocrítica adversarial

Antes de fechar, ataque o próprio relatório:

- As **três afirmações mais prováveis de estarem erradas**, e por quê.
- Onde você usou `[INFERIDO]` onde honestamente cabia `[DESCONHECIDO]`.
- Toda citação que não conseguiu verificar, listada nominalmente.
- Se você concluiu que os dados são irrecuperáveis: **qual observação o faria mudar de ideia?** Se
  concluiu que são recuperáveis: **qual o faria mudar de ideia?** Uma conclusão sem condição de
  refutação é opinião.

## 10. Formato de saída

Um documento com: sumário executivo de no máximo 15 linhas respondendo às duas perguntas da §0
diretamente; depois as Fases A–G, nessa ordem; depois um apêndice com todas as referências, cada
uma marcada verificada ou não verificada.

Escreva no registro do repositório: estado final, sem narrativa de processo, sem tabelas de
"diretrizes aplicadas", sem elogios à ferramenta e sem consolo. O leitor perdeu as fotografias da
própria infância e pediu certeza, não gentileza. A gentileza aqui é a precisão.

## 11. Proibições

- Não recomende ação que escreva na mídia de origem, sob nenhuma justificativa.
- Não conclua "provavelmente irrecuperável" como forma de encerrar a tarefa. Irrecuperabilidade é
  uma afirmação forte e exige a Fase E satisfeita.
- Não conclua "provavelmente recuperável" para consolar. Exige uma hipótese da Fase D sustentada por
  medição discriminante.
- Não trate ausência de achado como evidência de ausência sem a contabilidade de cobertura.
- Não invente números. Um número sem origem é pior que nenhum número.
