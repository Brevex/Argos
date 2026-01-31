# 🔮 Argos - Professional Image Recovery Tool

**Argos** é uma ferramenta profissional de recuperação forense de imagens escrita em Rust. Especializada em recuperar imagens JPEG e PNG de dispositivos de armazenamento, mesmo após múltiplas formatações.

## ✨ Características

- **Recuperação Profunda**: Recupera imagens mesmo de discos formatados dezenas de vezes
- **Zero-Overhead**: Arquitetura otimizada para máxima performance
- **Direct I/O**: Leitura direta do disco sem cache do sistema operacional
- **Resiliência**: Pula automaticamente setores defeituosos
- **Multi-formato**: Suporta JPEG e PNG
- **Interface Amigável**: CLI interativa com wizard guiado

## 🚀 Uso Rápido

### Modo Interativo (Recomendado)

```bash
sudo ./target/release/argos --scan
```

Isso abrirá um wizard interativo que:
1. Descobre todos os dispositivos de bloco disponíveis
2. Permite selecionar o dispositivo a ser analisado
3. Solicita o diretório de saída
4. Confirma a operação antes de iniciar

### Modo Linha de Comando

```bash
sudo ./target/release/argos --device /dev/sda --output ./recovered
```

## 📦 Instalação

### Pré-requisitos

- Rust 1.70+ (`rustup install stable`)
- Linux ou Windows
- Acesso root/administrador (para leitura de dispositivos de bloco)

### Compilação

```bash
# Clone o repositório
git clone https://github.com/seu-usuario/argos.git
cd argos

# Compile em modo release (otimizado)
cargo build --release

# O binário estará em target/release/argos
```

## 🔧 Arquitetura

O Argos utiliza uma arquitetura de pipeline eficiente:

```
┌─────────────┐    ┌─────────────┐    ┌─────────────┐    ┌─────────────┐
│   SCAN      │───▶│  ANALYZE    │───▶│   CARVE     │───▶│  EXTRACT    │
│  (I/O)      │    │  (CPU)      │    │  (CPU)      │    │  (I/O)      │
└─────────────┘    └─────────────┘    └─────────────┘    └─────────────┘
```

### Módulos

- **io**: Direct I/O com buffers alinhados (O_DIRECT no Linux)
- **analysis**: Cálculo de entropia e validação de assinaturas
- **carving**: Algoritmos de reconstrução (Linear, Bifragment)
- **extraction**: Escrita segura com fsync

## 📊 Performance

| Métrica | Target | Alcançado |
|---------|--------|-----------|
| Binary size | <5MB | ~900KB |
| Throughput SSD | >500 MB/s | ✓ |
| Memory footprint | <100MB/TB | ✓ |

## 🧪 Testes

```bash
# Executa todos os testes
cargo test

# Executa testes com output detalhado
cargo test -- --nocapture
```

## 📋 Algoritmos de Carving

### Linear Carving
Busca pares header→footer contíguos. O método mais rápido e confiável para arquivos não fragmentados.

### Bifragment Gap Carving
Para arquivos divididos em 2 fragmentos. Útil quando há dados corrompidos/sobrescritos entre header e footer.

## ⚠️ Avisos

1. **Execute como root**: Necessário para acessar dispositivos de bloco
2. **Operação somente-leitura**: O Argos NUNCA modifica o dispositivo de origem
3. **Espaço de saída**: Certifique-se de ter espaço suficiente para os arquivos recuperados

## 📝 Licença

MIT License - Veja [LICENSE](LICENSE) para detalhes.

## 🤝 Contribuindo

Contribuições são bem-vindas! Por favor:

1. Fork o repositório
2. Crie uma branch para sua feature (`git checkout -b feature/amazing`)
3. Commit suas mudanças (`git commit -m 'feat: add amazing feature'`)
4. Push para a branch (`git push origin feature/amazing`)
5. Abra um Pull Request

---

**Argos** - Recuperação de imagens com precisão forense 🔮
