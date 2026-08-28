# Argos — inventário de algoritmos

Todo algoritmo que o backend executa, com o nome publicado quando existe um. Onde não existe, a linha
diz. Cada âncora aponta para o código que o implementa. Complementa
[OVERVIEW.md §5](OVERVIEW.md), que traz a mesma lista sem âncoras.

| Técnica / algoritmo | Onde entra | Âncora |
| --- | --- | --- |
| **Aquisição multipasse estilo ddrescue** — varredura sequencial pulando falhas, depois refinamento setor a setor das regiões suspeitas | `argos acquire` | [acquire.rs:308](../crates/argos_device/src/acquire.rs#L308), [:374](../crates/argos_device/src/acquire.rs#L374) |
| **Leitura `O_DIRECT`** (contorna o page cache), com recurso a leitura cacheada | HAL Linux | [linux.rs:155](../crates/argos_device/src/device/linux.rs#L155) |
| **Parse MBR (PC/AT) e GPT UEFI** com validação **CRC-32** do cabeçalho e recurso ao GPT de backup | estágio A | [part.rs:67](../crates/argos_fs/src/part.rs#L67) |
| **Varredura de âncoras por fronteira de setor** (passo 512 B), validada por consistência interna e nunca por posição — *sem nome publicado; é a "residue sweep" deste projeto* | estágio A | [residue.rs:173](../crates/argos_fs/src/residue.rs#L173) |
| **Fixups do Update Sequence Array (USA)** — verificação e remoção | registros `FILE` NTFS | [ntfs.rs:1110](../crates/argos_fs/src/ntfs.rs#L1110) |
| **Decodificação de data runs** (runlist NTFS): par de nibbles + **delta de LCN assinado** | `$DATA` não-residente | [ntfs.rs:1167](../crates/argos_fs/src/ntfs.rs#L1167) |
| **Change journal `$UsnJrnl:$J`** (`USN_RECORD_V2`) | datas de exclusão | [ntfs.rs:422](../crates/argos_fs/src/ntfs.rs#L422) |
| **Mineração de slack de índice `$I30`** (buffers `INDX`) | nomes apagados | [ntfs.rs:1255](../crates/argos_fs/src/ntfs.rs#L1255) |
| **Caminhamento de árvore de extents ext4** (`ext4_extent_header`), profundidade ≤ 5 | extents de inode | [ext4.rs:391](../crates/argos_fs/src/ext4.rs#L391) |
| **Varredura do journal jbd2** por cópias antigas de blocos da tabela de inodes | o delete zera o extent tree no lugar; a cópia do journal não | [ext4.rs:298](../crates/argos_fs/src/ext4.rs#L298) |
| **Varredura de dirent apagado (`0xE5`)** + **reconstrução por hipótese de contiguidade** — a cadeia FAT foi zerada | FAT32/exFAT | [fat.rs:9](../crates/argos_fs/src/fat.rs#L9) |
| **Fletcher-64** — verificação de objeto | todo `obj_phys_t` APFS | [apfs.rs:494](../crates/argos_fs/src/apfs.rs#L494) |
| **Caminhamento de B-tree do object map (`omap`)** | resolução oid→bloco | [apfs.rs:482](../crates/argos_fs/src/apfs.rs#L482) |
| **Diff de checkpoints copy-on-write** | inodes que existiam e sumiram | [apfs.rs:104](../crates/argos_fs/src/apfs.rs#L104) |
| **CRC-32C** — selo de superbloco e de nó | btrfs | [btrfs.rs:174](../crates/argos_fs/src/btrfs.rs#L174) |
| **Caminhamento de B-tree** + **resolução lógico→físico pela chunk tree** | todo endereço btrfs | [btrfs.rs:607](../crates/argos_fs/src/btrfs.rs#L607) |
| **Busca de subcadeia Two-Way (Crochemore–Perrin)** com pré-filtro SIMD, via `memchr::memmem` | assinaturas `FF D8` e PNG | [lib.rs:29](../crates/argos_carve/src/lib.rs#L29) |
| **Máquina de estados JPEG** (T.81 Anexo B): SOI → segmentos → SOS → fluxo entrópico → EOI | validação | [jpeg.rs:45](../crates/argos_carve/src/jpeg.rs#L45) |
| **Máquina de estados PNG** com **CRC-32 por chunk** e **inflate zlib/DEFLATE incremental** (RFC 1950/1951) | validação | [png.rs:112](../crates/argos_carve/src/png.rs#L112) |
| **Walker TIFF/IFD** (EXIF 2.32) | miniatura embutida e dados de câmera | [exif.rs:1](../crates/argos_carve/src/exif.rs) |
| **Decodificação Huffman canônica** (T.81 §F.2.2.3) com **tabela de lookup direto** de 10 bits | oráculo, por hipótese | [mcu.rs:928](../crates/argos_carve/src/mcu.rs#L928) |
| **DPCM do coeficiente DC** (categoria + `EXTEND`, §F.2.2.1) e **run-length dos coeficientes AC** (`RRRRSSSS`, ZRL/EOB, §F.2.2.2) | por bloco 8×8 | [mcu.rs:1181](../crates/argos_carve/src/mcu.rs#L1181) |
| **De-stuffing `FF 00`** e verificação da cadência **`RSTn`** | fluxo entrópico | [mcu.rs:896](../crates/argos_carve/src/mcu.rs#L896) |
| **Decodificação incremental MCU a MCU** para localizar o ponto de fragmentação | estágio C → D | [reassemble.rs:285](../crates/argos_carve/src/reassemble.rs#L285) |
| **Retomada do decodificador** a partir do estado salvo na fronteira — uma hipótese custa os bytes que anexa, não o caminho inteiro | busca | [reassemble.rs:516](../crates/argos_carve/src/reassemble.rs#L516) |
| **Entropia de Shannon** por bloco de 4 KiB, sobre **histograma de 256 bins** | partição do espaço de busca | [classify.rs:192](../crates/argos_carve/src/classify.rs#L192) |
| **Detector de fluxo entrópico JPEG** por estatística de stuffing, e **detector de cabeçalho zlib/DEFLATE** (RFC 1950) | idem | [classify.rs:138](../crates/argos_carve/src/classify.rs#L138) |
| **Bifragment Gap Carving (BGC)** | dois fragmentos com um vão | [reassemble.rs:359](../crates/argos_carve/src/reassemble.rs#L359) |
| **Parallel Unique Path (PUP)** | mais de dois fragmentos | [reassemble.rs:732](../crates/argos_carve/src/reassemble.rs#L732) |
| **Busca em feixe (beam search) em profundidade** — largura 3 até profundidade 3, gulosa depois | passo do grafo | [reassemble.rs:1007](../crates/argos_carve/src/reassemble.rs#L1007) |
| **Poda por limite (branch and bound)** — ramo que não supera o alcance já obtido não é continuação | idem | [reassemble.rs:1007](../crates/argos_carve/src/reassemble.rs#L1007) |
| **Fusão de intervalos + busca binária** (consulta de predecessor) sobre os bytes já reivindicados | o que é espaço livre | [reassemble.rs:805](../crates/argos_carve/src/reassemble.rs#L805) |
| **Ordenação por proximidade**: busca binária pelo pivô + expansão *two-pointer* para os dois lados | ordem dos candidatos | [reassemble.rs:1138](../crates/argos_carve/src/reassemble.rs#L1138) |
| **MAD (mean absolute difference) de luminância entre linhas adjacentes, normalizada pela mediana do quadro** — variante do termo de *pixel-boundary smoothness* da literatura de remontagem | gate de costura | [decode.rs:73](../crates/argos_carve/src/decode.rs#L73) |
| **Enxerto de cabeçalho em ponto de reentrada `RSTn`** | fragmento órfão, fora do pipeline | [graft.rs](../crates/argos_engine/src/graft.rs) |
| **SHA-256** no momento da recuperação, e **dedup exata por digest** | proveniência e emissão | [output.rs:312](../crates/argos_engine/src/pipeline/output.rs#L312) |
| **Blockhash perceptual 8×8 (64 bits)** — média de bloco comparada à mediana | quase-duplicatas | [lib.rs:136](../crates/argos_classify/src/lib.rs#L136) |
| **Distância de Hamming** (≤ 3) | agrupamento de quase-duplicatas | [lib.rs:187](../crates/argos_classify/src/lib.rs#L187) |
| **Estatísticas determinísticas de triagem**: fração de alfa, contagem de cores distintas, fração de vizinhos idênticos (*flat runs*) e **segunda diferença horizontal de luminância** (o piso de alta frequência de um sensor) | rótulo foto vs asset | [rules.rs:230](../crates/argos_classify/src/rules.rs#L230) |
| **Detecção de runs de vizinhos do mesmo tamanho** | identificar cache de miniaturas | [finding.rs:578](../crates/argos_engine/src/finding.rs#L578) |
| **Ordenação total determinística + coalescência por cobertura de extents** | merge de findings entre estágios | [finding.rs:344](../crates/argos_engine/src/finding.rs#L344) |
