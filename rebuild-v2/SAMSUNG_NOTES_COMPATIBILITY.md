# Compatibilidad con Samsung Notes y PDF

## Que hace Samsung Notes

Samsung documenta dos comportamientos distintos:

- Una nota Samsung editable se comparte como archivo Samsung Notes (`.sdocx`).
- Un PDF se puede importar en Samsung Notes para escribir, dibujar o resaltar
  encima, pero el contenido original del PDF no se vuelve editable.

Fuentes oficiales:

- [Use Samsung Notes features and settings](https://www.samsung.com/us/support/answer/ANS10001384/)
- [Import and export PDFs with Samsung Notes](https://www.samsung.com/us/support/answer/ANS10002404/)
- [Samsung Philippines: import and export PDFs](https://www.samsung.com/ph/support/mobile-devices/how-to-import-and-export-pdfs-with-samsung-notes/)
- [Samsung Notes 2026: improved PDF export](https://us.community.samsung.com/t5/Global-Content/Notes-News-The-beginning-of-New-Notes/ba-p/3507625)

La actualizacion anunciada en 2026 mejora la definicion del PDF y el tratamiento
de resaltados. Samsung no anuncia que convierta el PDF exportado en el formato
editable de la nota. Su propia guia sigue indicando que en un PDF se pueden
anadir texto y dibujos, pero no editar su contenido original.

## Compatibilidad implementada

Inhouse Notes conserva el PDF original como fondo vectorial y mantiene los
trazos por encima como objetos independientes. Al importar un PDF:

1. El binario original se guarda una sola vez como asset, no dentro de cada
   pagina ni de cada autosave.
2. Las anotaciones PDF estandar de subtipo `/Ink` se convierten en trazos
   normales de Inhouse Notes.
3. Esos trazos pueden borrarse, deshacerse, rehacerse y combinarse con trazos
   nuevos.
4. Al exportar, los trazos se escriben como anotaciones `/Ink` estandar en vez
   de aplanarlos sobre la pagina.

El test `src/pdf/pdfRoundtrip.test.ts` comprueba el ciclo completo: exportar,
detectar `/Ink` con un lector independiente, reimportar y borrar el trazo.

## Limites del formato

- Si Samsung Notes exporta la escritura aplanada dentro del contenido grafico
  de la pagina, el PDF ya no contiene objetos de trazo. Se puede escribir y
  borrar lo nuevo, pero no separar con precision la tinta ya aplanada del fondo.
- `.sdocx` es un formato diferente de PDF. Esta version no intenta analizarlo
  directamente. Se puede exportar la nota desde Samsung Notes como PDF e
  importarla; la editabilidad de tinta previa dependera de que Samsung la haya
  conservado como anotaciones PDF.
- Formas, texto libre, sellos y otros subtipos de anotacion no se convierten aun
  a trazos. Permanecen visibles si forman parte del render del PDF.

## Prueba recomendada con archivos reales

Exportar la misma nota de Samsung Notes como PDF y como `.sdocx`, conservar
ambos originales y verificar en el PDF si la tinta aparece como anotaciones
`/Ink`. Esto permite distinguir una limitacion de exportacion de Samsung de un
fallo de importacion en Inhouse Notes.
