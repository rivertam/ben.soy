`umap-js-1.4.0.min.js` is the unmodified browser bundle from
[`umap-js` 1.4.0](https://www.npmjs.com/package/umap-js/v/1.4.0), maintained
at https://github.com/PAIR-code/umap-js. It is served locally so the map
does not depend on a CDN. `UMAP-LICENSE` is the package's Apache 2.0 license.

The worker supplies exact neighborhoods computed by Rust, a fixed random
seed, three output dimensions, 400 epochs, and `minDist = 0.3`. The layout
uses Euclidean distance between square roots of muscle weights divided by
each exercise's peak weight. This preserves proportional-profile equivalence
without forcing every profile to have unit length: a compound's additional
muscles do not dilute its shared support muscles. Movement tags label
groups but do not affect distances. After UMAP, 100 metric-MDS
SMACOF iterations fit the original distances between every pair, reducing
exaggerated inter-cluster gaps while starting from the local arrangement.
Rust supplies the upper triangle of the distance matrix, quantized to
four decimal places to keep the page payload compact. No movement or
exercise receives a manually chosen position or attraction.
Recommendations and the exact
muscle-match list continue to use the original muscle weights.
