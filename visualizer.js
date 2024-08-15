// Modify the number of grid rows and columns here:
initializeVisualizer(10, 5);

function visualize() {
    analyser.getByteFrequencyData(dataArray);

    const bandsPerCell = Math.floor(dataArray.length / (gridCells.length));
    gridCells.forEach((cell, index) => {
        const startFrequency = index * bandsPerCell;
        const endFrequency = startFrequency + bandsPerCell;
        const amplitude = dataArray.slice(startFrequency, endFrequency).reduce((a, b) => a + b, 0) / bandsPerCell;

        const color = new THREE.Color(`hsl(${(amplitude / 256) * 360}, 100%, 50%)`);
        cell.material.color = color;
    });

    renderer.render(scene, camera);
}

function animate() {
    requestAnimationFrame(animate);
    visualize();
}

animate();
