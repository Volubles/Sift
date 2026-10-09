# Sift

Sift är ett lokalt samplebibliotek för ljudeffekter, samples och musikfiler.
Målet är snabb sökning, smidig förhandslyssning och projekt där valda ljud kan
samlas och kopieras till andra mappar. Låg CPU- och minnesanvändning är ett
grundkrav.

Projektet är i förberedelsefasen. Ingen programkod eller körbar app finns ännu.
Rust är vald som huvudspråk. UI och ljudbibliotek ska verifieras i en liten
prototyp innan de låses för resten av implementationen.

Git-historiken ska bestå av små, sammanhängande Conventional Commits vid
naturliga checkpoints. Privata ljudbanker, databaser, exporter, mätresultat och
IDE-inställningar hör inte hemma i repot.

## Börja här

- [AGENTS.md](AGENTS.md): arbetsregler, kodkvalitet och verifiering.
- [Aktuell status](docs/STATUS.md): vad som faktiskt finns och nästa steg.
- [Produktkrav](docs/PRODUCT.md): funktioner, fasta regler och acceptans.
- [Arkitektur](docs/ARCHITECTURE.md): teknisk riktning och ansvarsgränser.
- [Prestanda](docs/PERFORMANCE.md): preliminära budgetar och mätmetod.
- [Git-konventioner](GIT-COMMIT-CONVENTIONS.md): små naturliga checkpoints.

Programmet kan ännu inte byggas eller köras. Run-/buildinstruktioner tillkommer
när det finns en verklig Cargo-implementation att verifiera.
