import cytoscape from 'cytoscape';
import fcose from 'cytoscape-fcose';

cytoscape.use(fcose);

self.onmessage = function ({ data: { elements, options } }) {
    const cy = cytoscape({ headless: true, elements });

    const layout = cy.layout({ name: 'fcose', ...options, animate: false });

    layout.one('layoutstop', () => {
        const positions = {};
        cy.nodes().forEach(n => { positions[n.id()] = n.position(); });
        self.postMessage({ positions });
        cy.destroy();
    });

    layout.run();
};
